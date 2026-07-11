use anyhow::{anyhow, Result};
use std::cell::RefCell;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use crate::config::PlayerKind;

pub trait Player {
    fn load(&mut self, path: &Path) -> Result<()>;
    /// Load `path` and begin playback at `start_secs` (no from-0 replay).
    /// Used by the premium-upgrade swap to resume the new stream at the
    /// captured position. Default falls back to `load` + `seek_to` so backends
    /// without a native start-offset load (afplay, stub) stay consistent.
    fn load_at(&mut self, path: &Path, start_secs: f64) -> Result<()> {
        self.load(path)?;
        self.seek_to(start_secs)
    }
    fn play_pause(&mut self) -> Result<()>;
    fn seek(&mut self, secs: f64) -> Result<()>;
    /// Seek to an absolute position (seconds from the track start). Used by
    /// `load_at`'s default fallback (load + seek_to) on backends without a
    /// native start-offset load. Default falls back to relative `seek` (which
    /// lands at the absolute target since a fresh load starts at 0) so afplay
    /// and the stub stay consistent.
    fn seek_to(&mut self, secs: f64) -> Result<()> {
        self.seek(secs)
    }
    fn stop(&mut self) -> Result<()>;
    /// Set playback volume as 0..=100. Best-effort: backends that can't
    /// (afplay has no IPC) return Ok(()) without doing anything, matching the
    /// existing `seek` no-op pattern on afplay. mpv implements this for real
    /// via the `volume` property.
    fn set_volume(&mut self, _vol: u8) -> Result<()> { Ok(()) }
    /// Mute/unmute. Best-effort like `set_volume`. mpv mutes via the `mute`
    /// property; the default is a no-op so afplay/stub stay consistent.
    fn set_muted(&mut self, _muted: bool) -> Result<()> { Ok(()) }
    fn position(&self) -> Option<f64>;
    fn duration(&self) -> Option<f64>;
    fn is_playing(&self) -> bool;
    /// True when the current track has finished playing on its own (mpv
    /// end-file with reason "eof", or afplay child exited). The TUI polls
    /// this each loop tick to auto-advance the queue. Default: no detection.
    fn track_ended(&mut self) -> bool { false }
}

// ---------- Stub (tests / dry-run) ----------
#[derive(Default)]
pub struct StubPlayer {
    loaded: Option<std::path::PathBuf>,
    playing: bool,
    pos: f64,
    dur: f64,
}
impl StubPlayer {
    pub fn loaded(&self) -> Option<std::path::PathBuf> { self.loaded.clone() }
}
impl Player for StubPlayer {
    fn load(&mut self, path: &Path) -> Result<()> { self.loaded = Some(path.to_path_buf()); self.playing = true; self.pos = 0.0; self.dur = 180.0; Ok(()) }
    fn play_pause(&mut self) -> Result<()> { self.playing = !self.playing; Ok(()) }
    fn seek(&mut self, secs: f64) -> Result<()> { self.pos = (self.pos + secs).max(0.0).min(self.dur); Ok(()) }
    fn stop(&mut self) -> Result<()> { self.playing = false; Ok(()) }
    fn position(&self) -> Option<f64> { Some(self.pos) }
    fn duration(&self) -> Option<f64> { Some(self.dur) }
    fn is_playing(&self) -> bool { self.playing }
}

// ---------- afplay fallback (per-track, no seek) ----------
#[derive(Default)]
pub struct AfplayPlayer { child: Option<RefCell<Child>>, paused: bool }
impl AfplayPlayer {
    pub fn new() -> Self { Self::default() }

    /// Kill + reap the current child (if any). Dropping a `Child` on Unix
    /// does NOT kill the process — it only orphans it, leaving afplay playing
    /// forever. Every load/stop must explicitly kill+reap, or rapid loads
    /// stack overlapping afplay processes (the "weird songs" bug).
    fn kill_current(&mut self) {
        if let Some(c) = self.child.take() {
            let mut child = c.into_inner();
            let _ = child.kill();
            let _ = child.wait();   // reap to avoid a zombie
        }
        self.paused = false;
    }

    /// Send a Unix signal to the afplay process. Used for SIGSTOP/SIGCONT
    /// pause/resume (afplay has no IPC). The pid is the direct child.
    #[cfg(unix)]
    fn signal_child(&self, sig: i32) {
        if let Some(c) = self.child.as_ref() {
            let pid = c.borrow().id() as i32;
            if pid > 0 {
                unsafe { libc::kill(pid, sig); }
            }
        }
    }

    #[cfg(not(unix))]
    fn signal_child(&self, _sig: i32) {}

}
impl Player for AfplayPlayer {
    fn load(&mut self, path: &Path) -> Result<()> {
        // Kill the previous afplay BEFORE spawning the next. Without this,
        // replacing `self.child` drops the old `Child` handle but leaves the
        // afplay process running — overlapping playback on rapid enters.
        self.kill_current();
        let child = Command::new("afplay")
            .arg(path)
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn()?;
        self.child = Some(RefCell::new(child));
        self.paused = false;
        Ok(())
    }
    fn play_pause(&mut self) -> Result<()> {
        // afplay has no IPC, so we pause it at the kernel level: SIGSTOP
        // freezes the process (audio stops immediately), SIGCONT resumes.
        // The track keeps its position because afplay's internal buffer
        // state is preserved across stop/cont.
        #[cfg(unix)]
        {
            if self.child.is_some() {
                if self.paused {
                    self.signal_child(libc::SIGCONT);
                    self.paused = false;
                } else {
                    self.signal_child(libc::SIGSTOP);
                    self.paused = true;
                }
            }
        }
        #[cfg(not(unix))]
        { let _ = self; }
        Ok(())
    }
    fn seek(&mut self, _secs: f64) -> Result<()> { Ok(()) }
    fn stop(&mut self) -> Result<()> {
        self.kill_current();
        Ok(())
    }
    fn position(&self) -> Option<f64> { None }
    fn duration(&self) -> Option<f64> { None }
    fn is_playing(&self) -> bool {
        // `Child::try_wait` needs `&mut`, so the child is wrapped in a RefCell
        // to allow this read probe through the trait's `&self` signature.
        self.child.as_ref().map(|c| c.borrow_mut().try_wait().ok().flatten().is_none()).unwrap_or(false)
    }
    fn track_ended(&mut self) -> bool {
        // afplay exits when the track finishes. If the child is present and
        // has exited on its own, reap it and report ended. `stop()` takes the
        // child out (None), so a manual stop won't fire this.
        if let Some(c) = self.child.as_ref() {
            if c.borrow_mut().try_wait().ok().flatten().is_some() {
                if let Some(c) = self.child.take() { let _ = c.into_inner().wait(); }
                self.paused = false;
                return true;
            }
        }
        false
    }
}
impl Drop for AfplayPlayer {
    fn drop(&mut self) {
        // Last-resort cleanup: if the App is dropped without an explicit
        // stop() (e.g. a panic mid-loop), kill afplay so it can't outlive
        // the TUI and keep playing "weird songs" after we exit.
        self.kill_current();
    }
}

// ---------- mpv over Unix socket ----------
pub struct MpvPlayer {
    child: RefCell<Child>,
    sock: std::path::PathBuf,
    conn: Option<std::os::unix::net::UnixStream>,
    /// Buffered IPC bytes that didn't end on a newline yet. mpv IPC is
    /// newline-delimited JSON, so a single read can split a line; we
    /// accumulate until a `\n`, then parse.
    line_buf: String,
    /// Latest polled values, updated from `observe_property` events. `None`
    /// until mpv first reports them (and reset to `None` on each load so a
    /// stale duration can't bleed into the next track).
    position: Option<f64>,
    duration: Option<f64>,
}
impl MpvPlayer {
    pub fn spawn(socket: &Path) -> Result<Self> {
        let _ = std::fs::remove_file(socket);
        // mpv's ao_coreaudio sets the device's HW format to the source file's
        // sample rate when it opens audio, so mpv naturally plays at the
        // file's rate (no resampling to 48k) — this is what keeps our
        // CoreAudio pre-switch from reverting. (We previously passed
        // --audio-resample=no, but that option does not exist in mpv and made
        // spawn fail fatally, silently forcing the afplay fallback — which in
        // turn broke pause and stacked orphaned afplay processes on rapid
        // loads.)
        let mut child = Command::new("mpv")
            .args(["--no-video", "--no-terminal", "--idle", "--gapless-audio=yes"])
            .arg(format!("--input-ipc-server={}", socket.display()))
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn()?;
        // wait for socket to appear (up to 2s)
        for _ in 0..20 {
            if socket.exists() { break; }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let conn = std::os::unix::net::UnixStream::connect(socket).ok();
        match conn {
            Some(conn) => {
                // Non-blocking so the TUI can poll for end-file events without
                // stalling the event loop.
                let _ = conn.set_nonblocking(true);
                let mut player = MpvPlayer {
                    child: RefCell::new(child),
                    sock: socket.to_path_buf(),
                    conn: Some(conn),
                    line_buf: String::new(),
                    position: None,
                    duration: None,
                };
                // Subscribe to time-pos + duration so the TUI can render a
                // progress bar without request/response round-trips. mpv
                // pushes a "property-change" event whenever the value
                // changes; we parse them in track_ended's read loop.
                let _ = player.send(&["observe_property".into(), 1.into(), "time-pos".into()]);
                let _ = player.send(&["observe_property".into(), 2.into(), "duration".into()]);
                Ok(player)
            }
            None => {
                // mpv IPC socket never appeared (or connect failed). Per spec
                // (mpv socket unavailable → afplay fallback), kill the child
                // and surface an error so `launch` falls back to AfplayPlayer.
                let _ = child.kill();
                let _ = child.wait();
                Err(anyhow!("mpv ipc socket unavailable at {}", socket.display()))
            }
        }
    }

    fn send(&mut self, cmd: &[serde_json::Value]) -> Result<()> {
        use std::io::Write;
        if let Some(c) = self.conn.as_mut() {
            let msg = serde_json::json!({ "command": cmd });
            writeln!(c, "{}", msg)?;
            c.flush()?;
        }
        Ok(())
    }

    /// Drain any buffered IPC data mpv has sent (event notifications + command
    /// responses) without blocking. Used to clear stale events when loading a
    /// new track so the replaced track's `end-file` doesn't fire auto-next.
    fn drain_socket(&mut self) {
        use std::io::Read;
        if let Some(c) = self.conn.as_mut() {
            let mut buf = [0u8; 4096];
            loop {
                match c.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => { /* keep draining */ }
                }
            }
        }
    }
}
impl Player for MpvPlayer {
    fn load(&mut self, path: &Path) -> Result<()> {
        // Fresh track → start at 0. `start` persists across files (a prior
        // `load_at` premium-swap may have left it at the old captured pos),
        // so reset it here.
        self.load_at(path, 0.0)
    }
    fn load_at(&mut self, path: &Path, start_secs: f64) -> Result<()> {
        // Drain any pending events for the track we're replacing so its
        // end-file doesn't trigger an auto-advance right after this load.
        self.drain_socket();
        // Reset cached playback state — the old track's duration/position
        // must not leak into the Now Playing panel before mpv reports the
        // new track's.
        self.position = None;
        self.duration = None;
        // mpv's `start` option seeks to a position at playback start, so the
        // new stream begins at `start_secs` directly — no from-0 replay before
        // a `seek` lands (the old load+seek path audibly restarted). It
        // persists across files, so `load` resets it to 0 for fresh tracks.
        let start_val = if start_secs.is_finite() && start_secs > 0.0 {
            format!("{}", start_secs)
        } else {
            "0".into()
        };
        self.send(&["set_property".into(), "start".into(), start_val.into()])?;
        self.send(&["loadfile".into(), path.to_string_lossy().into()])?;
        Ok(())
    }
    fn play_pause(&mut self) -> Result<()> {
        // mpv's `set` expects a boolean for the `pause` property; `cycle` toggles it.
        self.send(&["cycle".into(), "pause".into()])?;
        Ok(())
    }
    fn seek(&mut self, secs: f64) -> Result<()> {
        self.send(&["seek".into(), secs.into(), "relative".into()])?;
        Ok(())
    }
    fn seek_to(&mut self, secs: f64) -> Result<()> {
        // Absolute seek — used by the premium-upgrade swap to resume at the
        // captured position after `load` resets to 0. mpv supports an absolute
        // seek mode directly, which is cleaner than relative (no dependence on
        // the fresh load being at 0 when the command lands).
        self.send(&["seek".into(), secs.into(), "absolute".into()])?;
        Ok(())
    }
    fn set_volume(&mut self, vol: u8) -> Result<()> {
        // mpv's `volume` property is 0-100 (software volume, linear). Clamp
        // defensively — the TUI clamps too, but a stray value shouldn't
        // produce a nonsensical IPC command.
        let v = vol.clamp(0, 100) as f64;
        self.send(&["set_property".into(), "volume".into(), v.into()])?;
        Ok(())
    }
    fn set_muted(&mut self, muted: bool) -> Result<()> {
        self.send(&["set_property".into(), "mute".into(), muted.into()])?;
        Ok(())
    }
    fn stop(&mut self) -> Result<()> {
        let _ = self.send(&["quit".into()]);
        let mut child = self.child.borrow_mut();
        let _ = child.kill();
        let _ = child.wait();   // reap to avoid a zombie
        Ok(())
    }
    fn position(&self) -> Option<f64> { self.position }
    fn duration(&self) -> Option<f64> { self.duration }
    fn is_playing(&self) -> bool { self.child.borrow_mut().try_wait().ok().flatten().is_none() }
    fn track_ended(&mut self) -> bool {
        use std::io::Read;
        let Some(c) = self.conn.as_mut() else { return false };
        let mut tmp = [0u8; 8192];
        let mut ended = false;
        loop {
            match c.read(&mut tmp) {
                Ok(0) => break,        // socket closed (mpv quit)
                Ok(n) => {
                    // Accumulate and process complete newline-delimited JSON
                    // lines. A single read can span multiple events or split
                    // one, so we buffer by `\n`.
                    self.line_buf.push_str(&String::from_utf8_lossy(&tmp[..n]));
                    while let Some(idx) = self.line_buf.find('\n') {
                        let line: String = self.line_buf.drain(..=idx).collect();
                        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
                        let Some(ev) = v.get("event").and_then(|e| e.as_str()) else { continue };
                        match ev {
                            // property-change: update cached time-pos / duration.
                            "property-change" => {
                                let name = v.get("name").and_then(|n| n.as_str());
                                let data = v.get("data");
                                match name {
                                    Some("time-pos") => self.position = data.and_then(|d| d.as_f64()),
                                    Some("duration") => self.duration = data.and_then(|d| d.as_f64()),
                                    _ => {}
                                }
                            }
                            // end-file with reason "eof": the track finished
                            // naturally. "redirect" (replaced by loadfile) and
                            // "stop"/"quit" are ignored so manual skips don't
                            // double-advance the queue.
                            "end-file" if v.get("reason").and_then(|r| r.as_str()) == Some("eof") => {
                                ended = true;
                            }
                            _ => {}
                        }
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        ended
    }
}
impl Drop for MpvPlayer {
    fn drop(&mut self) {
        let mut child = self.child.borrow_mut();
        let _ = child.kill();
        let _ = child.wait();   // reap to avoid a zombie
        let _ = std::fs::remove_file(&self.sock);
    }
}

pub fn launch(kind: PlayerKind, socket: &Path) -> Box<dyn Player> {
    match kind {
        PlayerKind::Mpv => match MpvPlayer::spawn(socket) {
            Ok(p) => Box::new(p),
            Err(_) => Box::new(AfplayPlayer::new()),
        },
        PlayerKind::Afplay => Box::new(AfplayPlayer::new()),
    }
}
