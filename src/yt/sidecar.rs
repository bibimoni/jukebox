//! The Rust client for the Python sidecar: spawns `python script` as a
//! long-lived child process, writes newline-delimited [`Request`]s to its
//! stdin, and reads newline-delimited [`Response`]s from its stdout.
//!
//! **Non-blocking by design.** A thin reader thread blocks on the child's
//! stdout and pushes each complete line into an [`mpsc`] channel; [`try_recv`]
//! drains the channel without blocking the TUI poll loop (mirroring the
//! non-blocking read pattern already used for mpv IPC). The TUI reads results
//! on the next tick, never blocking the event loop.

use crate::yt::proto::{Request, Response};
use anyhow::{anyhow, Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;

pub struct Sidecar {
    child: Child,
    stdin: std::process::ChildStdin,
    rx: mpsc::Receiver<String>,
    /// Held (never joined) to keep the reader thread alive for the struct's
    /// lifetime; dropped (stopping the thread) when `Sidecar` drops.
    _reader: Option<std::thread::JoinHandle<()>>,
    cookies: Option<String>,
    browser: Option<String>,
    /// Persistent path the sidecar writes the decrypted browser cookies to
    /// (via JUKEBOX_YT_COOKIES_FILE env), so the next launch can load them
    /// without re-reading the Keychain. None for the pasted/guest path.
    cookies_file: Option<String>,
    python: std::path::PathBuf,
    script: std::path::PathBuf,
}

impl Sidecar {
    /// Spawn `python script` with auth passed to the sidecar:
    /// - `cookies` (Netscape cookies.txt) via the `JUKEBOX_YT_COOKIES` env var,
    ///   OR
    /// - `browser` (e.g. `"chrome"`) via `JUKEBOX_YT_BROWSER`, which makes the
    ///   sidecar read cookies from that browser's profile (yt-dlp's
    ///   `--cookies-from-browser` + browser_cookie3 for ytmusicapi). No cookie
    ///   file is written; values never leave the browser.
    /// Both `None` → guest mode.
    pub fn spawn(
        python: &Path,
        script: &Path,
        cookies: Option<String>,
        browser: Option<String>,
        cookies_file: Option<String>,
    ) -> Result<Self> {
        let mut cmd = Command::new(python);
        cmd.arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env("JUKEBOX_YT_COOKIES", cookies.clone().unwrap_or_default())
            .env("JUKEBOX_YT_BROWSER", browser.clone().unwrap_or_default())
            .env(
                "JUKEBOX_YT_COOKIES_FILE",
                cookies_file.clone().unwrap_or_default(),
            );
        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawning sidecar {} {}", python.display(), script.display()))?;
        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        let (tx, rx) = mpsc::channel::<String>();
        let reader = std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match r.read_line(&mut line) {
                    Ok(0) => break, // EOF — child closed stdout
                    Ok(_) => {
                        let l = line.trim().to_string();
                        if !l.is_empty() {
                            if tx.send(l).is_err() {
                                break; // receiver dropped
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Sidecar {
            child,
            stdin,
            rx,
            _reader: Some(reader),
            cookies,
            browser,
            cookies_file,
            python: python.to_path_buf(),
            script: script.to_path_buf(),
        })
    }

    /// Send a request. Writes one JSON line to the child's stdin + flush.
    pub fn send(&mut self, req: &Request) -> Result<()> {
        let line = req.to_line();
        writeln!(self.stdin, "{line}")?;
        self.stdin.flush()?;
        Ok(())
    }

    /// Non-blocking receive: `Ok(None)` if no complete response is ready,
    /// `Ok(Some(resp))` on a parsed line, `Err` if the sidecar has closed.
    pub fn try_recv(&mut self) -> Result<Option<Response>> {
        match self.rx.try_recv() {
            Ok(line) => Ok(Some(Response::from_line(&line)?)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => Err(anyhow!("sidecar closed")),
        }
    }

    /// True while the child process is still running. `try_wait` reaps on exit
    /// but is safe to call repeatedly; `None` means "still running".
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Best-effort restart once: kill the old child + reader, respawn with the
    /// same python/script/cookies/browser. Returns Err if the respawn fails.
    pub fn respawn(&mut self) -> Result<()> {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let new = Sidecar::spawn(&self.python, &self.script, self.cookies.clone(), self.browser.clone(), self.cookies_file.clone())?;
        *self = new;
        Ok(())
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
