use anyhow::Result;
use std::cell::RefCell;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use crate::config::PlayerKind;

pub trait Player {
    fn load(&mut self, path: &Path) -> Result<()>;
    fn play_pause(&mut self) -> Result<()>;
    fn seek(&mut self, secs: f64) -> Result<()>;
    fn stop(&mut self) -> Result<()>;
    fn position(&self) -> Option<f64>;
    fn duration(&self) -> Option<f64>;
    fn is_playing(&self) -> bool;
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
pub struct AfplayPlayer { child: Option<RefCell<Child>> }
impl AfplayPlayer {
    pub fn new() -> Self { Self { child: None } }
}
impl Player for AfplayPlayer {
    fn load(&mut self, path: &Path) -> Result<()> {
        self.child = Some(RefCell::new(Command::new("afplay").arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?));
        Ok(())
    }
    fn play_pause(&mut self) -> Result<()> { Ok(()) } // afplay has no IPC
    fn seek(&mut self, _secs: f64) -> Result<()> { Ok(()) }
    fn stop(&mut self) -> Result<()> {
        if let Some(c) = self.child.take() {
            let mut child = c.into_inner();
            let _ = child.kill();
            let _ = child.wait();   // reap to avoid a zombie
        }
        Ok(())
    }
    fn position(&self) -> Option<f64> { None }
    fn duration(&self) -> Option<f64> { None }
    fn is_playing(&self) -> bool {
        // `Child::try_wait` needs `&mut`, so the child is wrapped in a RefCell
        // to allow this read probe through the trait's `&self` signature.
        self.child.as_ref().map(|c| c.borrow_mut().try_wait().ok().flatten().is_none()).unwrap_or(false)
    }
}

// ---------- mpv over Unix socket ----------
pub struct MpvPlayer {
    child: RefCell<Child>,
    sock: std::path::PathBuf,
    conn: Option<std::os::unix::net::UnixStream>,
}
impl MpvPlayer {
    pub fn spawn(socket: &Path) -> Result<Self> {
        let _ = std::fs::remove_file(socket);
        let child = Command::new("mpv")
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
        Ok(MpvPlayer { child: RefCell::new(child), sock: socket.to_path_buf(), conn })
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
}
impl Player for MpvPlayer {
    fn load(&mut self, path: &Path) -> Result<()> {
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
    fn stop(&mut self) -> Result<()> {
        let _ = self.send(&["quit".into()]);
        let mut child = self.child.borrow_mut();
        let _ = child.kill();
        let _ = child.wait();   // reap to avoid a zombie
        Ok(())
    }
    fn position(&self) -> Option<f64> { None }   // polled in TUI via get_property (future)
    fn duration(&self) -> Option<f64> { None }
    fn is_playing(&self) -> bool { self.child.borrow_mut().try_wait().ok().flatten().is_none() }
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
