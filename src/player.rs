use anyhow::{anyhow, Result};
use std::cell::RefCell;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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
    fn set_volume(&mut self, _vol: u8) -> Result<()> {
        Ok(())
    }
    /// Mute/unmute. Best-effort like `set_volume`. mpv mutes via the `mute`
    /// property; the default is a no-op so afplay/stub stay consistent.
    fn set_muted(&mut self, _muted: bool) -> Result<()> {
        Ok(())
    }
    fn position(&self) -> Option<f64>;
    fn duration(&self) -> Option<f64>;
    fn is_playing(&self) -> bool;
    /// True when the current track has finished playing on its own (mpv
    /// end-file with reason "eof", or afplay child exited). The TUI polls
    /// this each loop tick to auto-advance the queue. Default: no detection.
    fn track_ended(&mut self) -> bool {
        false
    }
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
    pub fn loaded(&self) -> Option<std::path::PathBuf> {
        self.loaded.clone()
    }
}
impl Player for StubPlayer {
    fn load(&mut self, path: &Path) -> Result<()> {
        self.loaded = Some(path.to_path_buf());
        self.playing = true;
        self.pos = 0.0;
        self.dur = 180.0;
        Ok(())
    }
    fn play_pause(&mut self) -> Result<()> {
        self.playing = !self.playing;
        Ok(())
    }
    fn seek(&mut self, secs: f64) -> Result<()> {
        self.pos = (self.pos + secs).max(0.0).min(self.dur);
        Ok(())
    }
    fn stop(&mut self) -> Result<()> {
        self.playing = false;
        Ok(())
    }
    fn position(&self) -> Option<f64> {
        Some(self.pos)
    }
    fn duration(&self) -> Option<f64> {
        Some(self.dur)
    }
    fn is_playing(&self) -> bool {
        self.playing
    }
}

// ---------- afplay fallback (per-track, no seek) ----------
#[derive(Default)]
pub struct AfplayPlayer {
    child: Option<RefCell<Child>>,
    paused: bool,
}
impl AfplayPlayer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Kill + reap the current child (if any). Dropping a `Child` on Unix
    /// does NOT kill the process — it only orphans it, leaving afplay playing
    /// forever. Every load/stop must explicitly kill+reap, or rapid loads
    /// stack overlapping afplay processes (the "weird songs" bug).
    fn kill_current(&mut self) {
        if let Some(c) = self.child.take() {
            let mut child = c.into_inner();
            let _ = child.kill();
            let _ = child.wait(); // reap to avoid a zombie
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
                unsafe {
                    libc::kill(pid, sig);
                }
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
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
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
        {
            let _ = self;
        }
        Ok(())
    }
    fn seek(&mut self, _secs: f64) -> Result<()> {
        Ok(())
    }
    fn stop(&mut self) -> Result<()> {
        self.kill_current();
        Ok(())
    }
    fn position(&self) -> Option<f64> {
        None
    }
    fn duration(&self) -> Option<f64> {
        None
    }
    fn is_playing(&self) -> bool {
        // RC11-DEF-003: a SIGSTOP-paused afplay child is still "running" from
        // the kernel's view (`try_wait` returns `Ok(None)`), so the old check
        // (`try_wait().is_none()`) reported `true` while paused — the player
        // bar kept showing `[PLAYING]` instead of `[PAUSED]`. Consult the
        // `paused` flag set by `play_pause` so the label reflects the real
        // state. `track_ended` still reaps a naturally-exited child via its
        // own `try_wait` probe (not gated on `is_playing`), so a paused track
        // is not mistaken for a finished one.
        self.child.is_some() && !self.paused
    }
    fn track_ended(&mut self) -> bool {
        // afplay exits when the track finishes. If the child is present and
        // has exited on its own, reap it and report ended. `stop()` takes the
        // child out (None), so a manual stop won't fire this.
        if let Some(c) = self.child.as_ref() {
            if c.borrow_mut().try_wait().ok().flatten().is_some() {
                if let Some(c) = self.child.take() {
                    let _ = c.into_inner().wait();
                }
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
    /// RC11-DEF-003: mpv's `pause` property, mirrored locally so
    /// `is_playing()` can report the paused state without a synchronous IPC
    /// round-trip. Toggled optimistically in `play_pause` and corrected by the
    /// `pause` property-change events drained in `track_ended`.
    paused: bool,
    /// RB-5: true after a successful load, cleared on stop / end. Lets
    /// `track_ended` treat a dead child (mpv exited without an eof event) as
    /// an implicit end so auto-advance fires.
    loaded: bool,
    /// RB-5: respawn backoff counter (≤3 attempts, ≥5s apart) so a persistently
    /// dying mpv doesn't cause a rapid respawn loop through the whole catalog.
    respawn_attempts: u32,
    last_respawn: Option<Instant>,
}
/// RB-5: Max auto-respawn attempts before giving up (surfacing the error so
/// the dead-track loop in `next()` skips the track instead of retrying mpv
/// spawn for every catalog entry).
const MPV_RESPAWN_MAX: u32 = 3;
/// RB-5: Minimum gap between respawn attempts.
const MPV_RESPAWN_GAP: Duration = Duration::from_secs(5);

impl MpvPlayer {
    pub fn spawn(socket: &Path) -> Result<Self> {
        let _ = std::fs::remove_file(socket);
        let (child, conn) = Self::spawn_child(socket)?;
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
            paused: false,
            loaded: false,
            respawn_attempts: 0,
            last_respawn: None,
        };
        player.subscribe_properties();
        Ok(player)
    }

    /// Spawn an mpv child with the IPC socket and connect to it. Shared by
    /// `spawn` (initial) and `respawn` (recovery).
    fn spawn_child(socket: &Path) -> Result<(Child, std::os::unix::net::UnixStream)> {
        // mpv's ao_coreaudio sets the device's HW format to the source file's
        // sample rate when it opens audio, so mpv naturally plays at the
        // file's rate (no resampling to 48k) — this is what keeps our
        // CoreAudio pre-switch from reverting. (We previously passed
        // --audio-resample=no, but that option does not exist in mpv and made
        // spawn fail fatally, silently forcing the afplay fallback — which in
        // turn broke pause and stacked orphaned afplay processes on rapid
        // loads.)
        let mut child = Command::new("mpv")
            .args([
                "--no-video",
                "--no-terminal",
                "--idle",
                "--gapless-audio=yes",
            ])
            .arg(format!("--input-ipc-server={}", socket.display()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        // wait for socket to appear (up to 4s — generous for CI/parallel load;
        // real use creates it in <500ms, this only bounds the afplay fallback).
        for _ in 0..40 {
            if socket.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        match std::os::unix::net::UnixStream::connect(socket) {
            Ok(conn) => Ok((child, conn)),
            Err(_) => {
                // mpv IPC socket never appeared (or connect failed). Per spec
                // (mpv socket unavailable → afplay fallback), kill the child
                // and surface an error so `launch` falls back to AfplayPlayer.
                let _ = child.kill();
                let _ = child.wait();
                Err(anyhow!(
                    "mpv ipc socket unavailable at {}",
                    socket.display()
                ))
            }
        }
    }

    /// Subscribe to time-pos + duration + pause so the TUI can render a
    /// progress bar and report pause state without request/response
    /// round-trips. mpv pushes a "property-change" event whenever a value
    /// changes; we parse them in track_ended's read loop.
    fn subscribe_properties(&mut self) {
        let _ = self.send(&["observe_property".into(), 1.into(), "time-pos".into()]);
        let _ = self.send(&["observe_property".into(), 2.into(), "duration".into()]);
        // RC11-DEF-003: observe the `pause` property so `is_playing()`
        // mirrors mpv's true pause state (corrects the optimistic toggle in
        // `play_pause` if mpv pauses/unpauses for any other reason).
        let _ = self.send(&["observe_property".into(), 3.into(), "pause".into()]);
    }

    /// RB-5: True when the mpv child has exited (eof exit, crash, or prior
    /// `stop()`). `try_wait` reaps on exit but is safe to call repeatedly.
    fn child_dead(&self) -> bool {
        self.child.borrow_mut().try_wait().ok().flatten().is_some()
    }

    /// RB-5: Respawn mpv after the child died / socket broke. Kills + reaps
    /// the old child, removes the stale socket, spawns a fresh mpv, connects
    /// a new IPC socket, and re-subscribes to property events. Rate-limited
    /// by a backoff (≤3 attempts, ≥5s apart) so a persistently dying mpv
    /// doesn't loop through the whole catalog spawning processes.
    fn respawn(&mut self) -> Result<()> {
        if self.respawn_attempts >= MPV_RESPAWN_MAX {
            if let Some(t) = self.last_respawn {
                if t.elapsed() < MPV_RESPAWN_GAP {
                    return Err(anyhow!(
                        "mpv respawn backoff ({} attempts)",
                        self.respawn_attempts
                    ));
                }
            }
        }
        let mut child = self.child.borrow_mut();
        let _ = child.kill();
        let _ = child.wait();
        drop(child);
        let _ = std::fs::remove_file(&self.sock);
        let (new_child, conn) = Self::spawn_child(&self.sock)?;
        let _ = conn.set_nonblocking(true);
        *self.child.borrow_mut() = new_child;
        self.conn = Some(conn);
        self.line_buf.clear();
        self.position = None;
        self.duration = None;
        self.paused = false;
        self.loaded = false;
        self.subscribe_properties();
        self.respawn_attempts += 1;
        self.last_respawn = Some(Instant::now());
        Ok(())
    }

    /// RB-5: Reset the respawn backoff after a successful load (mpv is alive
    /// and playing, so a future death is a fresh incident).
    fn mark_alive(&mut self) {
        self.respawn_attempts = 0;
        self.last_respawn = None;
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
        // RB-5: if the mpv child has exited (after eof, crash, or prior
        // stop()), respawn it before sending so the load recovers instead of
        // hitting a dead socket ("Broken pipe (os error 32)").
        if self.child_dead() {
            self.respawn()?;
        }
        // Drain any pending events for the track we're replacing so its
        // end-file doesn't trigger an auto-advance right after this load.
        self.drain_socket();
        // Reset cached playback state — the old track's duration/position
        // must not leak into the Now Playing panel before mpv reports the
        // new track's.
        self.position = None;
        self.duration = None;
        // RC11-DEF-003: a fresh load starts playing (mpv `loadfile` un-pauses
        // by default), so clear the mirrored pause flag so `is_playing()`
        // reports `[PLAYING]` until a property-change event says otherwise.
        self.paused = false;
        // mpv's `start` option seeks to a position at playback start, so the
        // new stream begins at `start_secs` directly — no from-0 replay before
        // a `seek` lands (the old load+seek path audibly restarted). It
        // persists across files, so `load` resets it to 0 for fresh tracks.
        let start_val = if start_secs.is_finite() && start_secs > 0.0 {
            format!("{}", start_secs)
        } else {
            "0".into()
        };
        match self
            .send(&[
                "set_property".into(),
                "start".into(),
                start_val.clone().into(),
            ])
            .and_then(|_| self.send(&["loadfile".into(), path.to_string_lossy().into()]))
        {
            Ok(()) => {
                self.loaded = true;
                self.mark_alive();
                Ok(())
            }
            Err(_) => {
                // RB-5: broken pipe / connection reset on a stale socket.
                // Respawn mpv + retry the load once.
                self.respawn()?;
                self.send(&["set_property".into(), "start".into(), start_val.into()])?;
                self.send(&["loadfile".into(), path.to_string_lossy().into()])?;
                self.loaded = true;
                self.mark_alive();
                Ok(())
            }
        }
    }
    fn play_pause(&mut self) -> Result<()> {
        // mpv's `set` expects a boolean for the `pause` property; `cycle` toggles it.
        // RC11-DEF-003: mirror the toggle locally so `is_playing()` can report
        // the paused state without waiting for the `pause` property-change
        // event to arrive (the event corrects this if it disagrees).
        self.paused = !self.paused;
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
        let _ = child.wait(); // reap to avoid a zombie
        self.loaded = false;
        Ok(())
    }
    fn position(&self) -> Option<f64> {
        self.position
    }
    fn duration(&self) -> Option<f64> {
        self.duration
    }
    fn is_playing(&self) -> bool {
        // RC11-DEF-003: the mpv child stays alive while paused (`--idle`), so
        // `try_wait().is_none()` alone reports `true` when paused. Consult the
        // mirrored `pause` flag so the player bar shows `[PAUSED]`.
        self.child.borrow_mut().try_wait().ok().flatten().is_none() && !self.paused
    }
    fn track_ended(&mut self) -> bool {
        use std::io::Read;
        let Some(c) = self.conn.as_mut() else {
            // RB-5: no connection but a track was loaded → child died → ended.
            if self.loaded && self.child_dead() {
                self.loaded = false;
                return true;
            }
            return false;
        };
        let mut tmp = [0u8; 8192];
        let mut ended = false;
        loop {
            match c.read(&mut tmp) {
                Ok(0) => {
                    // RB-5: socket closed (mpv quit). If a track was loaded
                    // and the child has exited, treat it as an implicit end
                    // so auto-advance fires (mpv exited without an eof event).
                    if !ended && self.loaded && self.child_dead() {
                        ended = true;
                    }
                    break;
                }
                Ok(n) => {
                    // Accumulate and process complete newline-delimited JSON
                    // lines. A single read can span multiple events or split
                    // one, so we buffer by `\n`.
                    self.line_buf.push_str(&String::from_utf8_lossy(&tmp[..n]));
                    while let Some(idx) = self.line_buf.find('\n') {
                        let line: String = self.line_buf.drain(..=idx).collect();
                        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                            continue;
                        };
                        let Some(ev) = v.get("event").and_then(|e| e.as_str()) else {
                            continue;
                        };
                        match ev {
                            // property-change: update cached time-pos / duration.
                            "property-change" => {
                                let name = v.get("name").and_then(|n| n.as_str());
                                let data = v.get("data");
                                match name {
                                    Some("time-pos") => {
                                        self.position = data.and_then(|d| d.as_f64())
                                    }
                                    Some("duration") => {
                                        self.duration = data.and_then(|d| d.as_f64())
                                    }
                                    // RC11-DEF-003: mirror mpv's `pause`
                                    // property so `is_playing()` reflects the
                                    // real state (corrects the optimistic
                                    // toggle in `play_pause` and tracks
                                    // external pause changes).
                                    Some("pause") => {
                                        self.paused =
                                            data.and_then(|d| d.as_bool()).unwrap_or(self.paused)
                                    }
                                    _ => {}
                                }
                            }
                            // end-file with reason "eof": the track finished
                            // naturally. "redirect" (replaced by loadfile) and
                            // "stop"/"quit" are ignored so manual skips don't
                            // double-advance the queue.
                            "end-file"
                                if v.get("reason").and_then(|r| r.as_str()) == Some("eof") =>
                            {
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
        if ended {
            self.loaded = false;
        }
        ended
    }
}
impl Drop for MpvPlayer {
    fn drop(&mut self) {
        let mut child = self.child.borrow_mut();
        let _ = child.kill();
        let _ = child.wait(); // reap to avoid a zombie
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

#[cfg(test)]
mod tests {
    use super::*;

    /// RC11-DEF-003: `AfplayPlayer::is_playing()` must consult `self.paused`,
    /// not just `try_wait().is_none()`. A SIGSTOP-paused afplay child is still
    /// running from the kernel's view, so the old check reported `true` while
    /// paused — the player bar kept showing `[PLAYING]`. With the fix, a
    /// paused child reports `is_playing() == false`.
    #[test]
    fn def003_afplay_is_playing_consults_paused_flag() {
        let mut p = AfplayPlayer::new();
        // No child → not playing regardless of `paused`.
        assert!(!p.is_playing(), "no child: not playing");
        // Spawn a long-running child (simulates afplay) and pause it.
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        p.child = Some(RefCell::new(child));
        p.paused = false;
        assert!(p.is_playing(), "child running + not paused: playing");
        // SIGSTOP-paused child: still running, but `paused` is true → not playing.
        p.paused = true;
        assert!(
            !p.is_playing(),
            "DEF-003: paused child must report is_playing() == false so the bar shows [PAUSED]"
        );
        // Resume: paused=false → playing again.
        p.paused = false;
        assert!(p.is_playing(), "resumed child: playing");
        // Clean up so the sleep child doesn't outlive the test.
        p.kill_current();
    }

    /// RC11-DEF-003: `MpvPlayer::is_playing()` must consult `self.paused`.
    /// The mpv child stays alive while paused (`--idle`), so `try_wait()`
    /// alone reports `true` when paused. With the fix, a paused mpv reports
    /// `is_playing() == false`. Constructs the struct directly (no socket /
    /// mpv binary needed) since the fields are accessible from the same
    /// module.
    #[test]
    fn def003_mpv_is_playing_consults_paused_flag() {
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let mut p = MpvPlayer {
            child: RefCell::new(child),
            sock: std::path::PathBuf::from("/tmp/jk-test-mpv-sock-dummy"),
            conn: None,
            line_buf: String::new(),
            position: None,
            duration: None,
            paused: false,
            loaded: false,
            respawn_attempts: 0,
            last_respawn: None,
        };
        assert!(p.is_playing(), "mpv child running + not paused: playing");
        p.paused = true;
        assert!(
            !p.is_playing(),
            "DEF-003: paused mpv must report is_playing() == false so the bar shows [PAUSED]"
        );
        p.paused = false;
        assert!(p.is_playing(), "resumed mpv: playing");
        // Reap the sleep child so it doesn't outlive the test.
        let mut child = p.child.borrow_mut();
        let _ = child.kill();
        let _ = child.wait();
    }

    /// RB-5: `MpvPlayer::child_dead()` must report true after the child
    /// exits, so `load_at` respawns before sending (instead of hitting a
    /// dead socket with "Broken pipe (os error 32)").
    #[test]
    fn rb5_child_dead_detects_exited_child() {
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let p = MpvPlayer {
            child: RefCell::new(child),
            sock: std::path::PathBuf::from("/tmp/jk-test-mpv-sock-dummy-rb5"),
            conn: None,
            line_buf: String::new(),
            position: None,
            duration: None,
            paused: false,
            loaded: false,
            respawn_attempts: 0,
            last_respawn: None,
        };
        assert!(!p.child_dead(), "running child: not dead");
        // Kill + reap → child_dead must return true.
        {
            let mut c = p.child.borrow_mut();
            let _ = c.kill();
            let _ = c.wait();
        }
        assert!(p.child_dead(), "exited child: dead");
    }

    /// RB-5: `track_ended` must return true when the mpv child has exited
    /// without sending an eof event (crash / abrupt exit) AND a track was
    /// loaded. This lets auto-advance fire so `next()` → `load_track()` →
    /// `load_at()` respawns mpv. After `stop()`, `loaded` is false so this
    /// does NOT fire (no spurious auto-restart after a user stop).
    #[test]
    fn rb5_track_ended_detects_dead_child_after_load() {
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let mut p = MpvPlayer {
            child: RefCell::new(child),
            sock: std::path::PathBuf::from("/tmp/jk-test-mpv-sock-dummy-rb5-te"),
            conn: None,
            line_buf: String::new(),
            position: None,
            duration: None,
            paused: false,
            loaded: true,
            respawn_attempts: 0,
            last_respawn: None,
        };
        // Kill the child (simulates mpv exit) — no conn → dead-child path.
        {
            let mut c = p.child.borrow_mut();
            let _ = c.kill();
            let _ = c.wait();
        }
        assert!(
            p.track_ended(),
            "RB-5: track_ended must fire when child dies after load"
        );
        assert!(!p.loaded, "loaded must be cleared after ended");

        // After stop(), loaded is false → track_ended must NOT fire.
        p.loaded = false;
        assert!(
            !p.track_ended(),
            "RB-5: track_ended must not fire after stop (loaded false)"
        );
    }

    /// RB-5: `MpvPlayer::load` (via `load_at`) must respawn a dead mpv
    /// child and succeed, instead of yielding "Broken pipe (os error 32)".
    /// This is the core fix: after natural completion (mpv child exited),
    /// the next track loads by respawning mpv. Requires the `mpv` binary.
    #[test]
    fn rb5_mpv_load_respawns_dead_child() {
        if std::process::Command::new("mpv")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("skipping rb5_mpv_load_respawns_dead_child (no mpv binary)");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("mpv-rb5-sock");
        let mut player = MpvPlayer::spawn(&sock).expect("mpv spawn");
        assert!(!player.child_dead(), "freshly spawned mpv must be alive");

        // Simulate the child dying (eof exit / crash / prior stop).
        {
            let mut c = player.child.borrow_mut();
            let _ = c.kill();
            let _ = c.wait();
        }
        assert!(player.child_dead(), "killed child must be dead");

        // load must detect the dead child, respawn, and succeed.
        let dummy = dir.path().join("dummy.flac");
        std::fs::write(&dummy, b"x").unwrap();
        let result = player.load(&dummy);
        assert!(
            result.is_ok(),
            "RB-5: load after dead child must respawn + succeed: {:?}",
            result
        );
        assert!(
            !player.child_dead(),
            "RB-5: mpv child must be alive after respawn"
        );
        assert!(
            player.loaded,
            "RB-5: loaded must be true after successful load"
        );
        assert_eq!(
            player.respawn_attempts, 0,
            "RB-5: mark_alive must reset backoff after successful respawn"
        );
    }
}
