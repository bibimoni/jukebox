//! The terminal event loop.
//!
//! [`run`] is the single entry point: it puts the terminal into raw + alt-
//! screen + mouse-capture mode, installs a crash-safe panic hook and a SIGTSTP/
//! SIGCONT suspend dance, then spins on `crossterm::event::poll` dispatching
//! keys/mouse to [`crate::tui::input`] and redraws via
//! [`crate::tui::view::layout::draw`] until `app.should_quit`.
//!
//! ## Terminal hygiene (spec §"Terminal hygiene")
//!
//! - **Alternate screen** so scrollback is unpolluted.
//! - **Panic-safe restore.** A panic hook (and a [`TerminalGuard`] dropped on
//!   every exit path) disable raw mode, leave the alt screen, restore the
//!   cursor, restore the captured CoreAudio output format, and stop the player
//!   before the panic message prints. The hook is re-entrant: it chains to the
//!   previous hook.
//! - **SIGTSTP suspend.** A flag-based handler restores the terminal then raises
//!   `SIGTSTP` with its default disposition (so the shell suspends us); on
//!   `SIGCONT` a redraw is forced.
//! - **File logging** (no `eprintln!` in this module) via [`log_to_file`].

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use anyhow::{Context as _, Result};
use crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, queue};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::audio::{restore_output_format, CapturedFormat};
use crate::tui::app::App;
use crate::tui::{input, view};

/// Poll timeout for `crossterm::event::poll`. Long enough that the loop idles
/// lightly when there's no input, short enough that `track_ended` polls and
/// SIGTSTP/SIGCONT flags are checked at a responsive cadence.
const POLL_TIMEOUT_MS: u64 = 150;

/// Global slot holding the captured CoreAudio format, so the panic hook (which
/// has no closure capture of `run`'s locals) can restore it. Set once at the
/// top of [`run`] before the hook is installed; taken back out by the normal
/// exit path. On non-macOS this is always `None` and the restores are no-ops.
static CAPTURED: OnceLock<Mutex<Option<CapturedFormat>>> = OnceLock::new();

/// Set by the `SIGTSTP` handler; the loop drains it by restoring the terminal
/// and re-raising SIGTSTP with its default disposition.
static SIGTSTP_RECEIVED: AtomicBool = AtomicBool::new(false);

/// Set by the `SIGCONT` handler; the loop drains it by forcing a redraw.
static NEED_REDRAW: AtomicBool = AtomicBool::new(false);

/// A guard that restores the terminal on drop — covers the normal-exit path
/// and any early `?`-return inside [`run`] after the guard is constructed.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        // Best-effort cleanup; every error is ignored because we're already on
        // an exit path and the one invariant that matters is "don't strand the
        // terminal in raw + alt-screen mode".
        let _ = cleanup_terminal();
        cleanup_audio();
    }
}

/// Disable mouse capture, leave alt screen, disable raw mode, restore cursor.
fn cleanup_terminal() -> Result<()> {
    let mut stdout = std::io::stdout();
    queue!(stdout, crossterm::event::DisableMouseCapture)?;
    queue!(stdout, LeaveAlternateScreen)?;
    let _ = disable_raw_mode();
    execute!(stdout, crossterm::cursor::Show)?;
    Ok(())
}

/// Restore the captured CoreAudio format from the global slot (if any).
fn cleanup_audio() {
    if let Some(m) = CAPTURED.get() {
        if let Ok(mut guard) = m.lock() {
            if let Some(fmt) = guard.take() {
                restore_output_format(Some(fmt));
            }
        }
    }
}

/// Append a single (redacted) log line to `~/.cache/jukebox/jukebox.log` (or
/// the platform cache dir). Best-effort: a failed write is silently dropped
/// (we're often in the alt screen, where `eprintln!` would corrupt the UI
/// anyway). This is the `eprintln!` replacement for paths that originate in
/// the event loop.
///
/// **Rotation:** before writing, if the log exceeds 1 MiB it's rolled to
/// `jukebox.log.1` (overwriting any prior `.1`) and a fresh log is started —
/// a one-level rotation that keeps the log bounded without losing the most
/// recent history. Callers should pass the line through [`redact`] first so
/// cookie/token secrets never reach disk.
fn log_to_file(line: &str) {
    let Some(cache) = dirs::cache_dir() else {
        return;
    };
    let log_dir = cache.join("jukebox");
    let _ = std::fs::create_dir_all(&log_dir);
    let path = log_dir.join("jukebox.log");
    // One-level rotation: keep the log from growing without bound. A >1 MiB
    // log rolls to `.1` (clobbering the previous roll) so the active log
    // always holds the most recent ~1 MiB of lines.
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 1024 * 1024 {
            let _ = std::fs::rename(&path, log_dir.join("jukebox.log.1"));
        }
    }
    log_to_file_at(&path, line);
}

/// Append `line` to `path` (best-effort: a failed write is silently dropped).
/// Public so tests can write to a temp path instead of the real cache dir.
/// Creates the parent directory if missing. No rotation: rotation is a
/// property of the default cache log (see [`log_to_file`]).
pub fn log_to_file_at(path: &std::path::Path, line: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{line}");
    }
}

/// Redact cookie/token secrets from a log line before it's written to disk.
/// Replaces any `<marker><value>` substring with `[REDACTED]`, where `<marker>`
/// is one of `SAPISID=`, `__Secure-3PAPISID=`, `authorization=`, `cookie=`
/// (case-insensitive). The marker itself is consumed so the secret name
/// doesn't leak either; other text (including the surrounding context) is
/// preserved so the log stays readable. Public so the run loop can call it
/// before [`log_to_file`] and so tests can verify the redaction directly.
///
/// **Per-marker consume.** Most markers (`SAPISID=`, `SID=`, …) take a
/// single-token value (`[A-Za-z0-9_.-]*`), so the value is consumed up to the
/// first non-token char (whitespace, `;`, …) and the next marker on the same
/// line can still be caught. The `authorization=` and `cookie=` markers take
/// a value that may contain spaces and `=` (e.g. `Bearer <token>` or
/// `name=val; name2=val2`), so for those the ENTIRE remainder of the line is
/// consumed — otherwise `authorization=Bearer secret` would stop at the space
/// and leak `secret`.
pub fn redact(line: &str) -> String {
    /// Markers whose following value is a secret. Order doesn't matter: at a
    /// given position only one marker can match (they have distinct first
    /// chars), so a `find` over the table is unambiguous.
    const MARKERS: &[&str] = &[
        "__Secure-3PAPISID=",
        "__Secure-3PSID=",
        "SAPISID=",
        "APISID=",
        "SSID=",
        "SID=",
        "HSID=",
        "SIDCC=",
        "authorization=",
        "cookie=",
    ];
    /// Markers whose value may contain spaces / `=` / `;` (a full header or
    /// cookie line). For these the ENTIRE remainder of the line is the
    /// credential — consume to end-of-line so a `Bearer <token>` doesn't leak
    /// the token after the space, and a `cookie=name=val; …` line doesn't
    /// leak everything after the first `=`. The SID-family markers are single
    /// tokens (alnum + `_.-`), so they keep the default token consume.
    const CONSUME_TO_EOL: &[&str] = &["authorization=", "cookie="];
    let mut out = String::with_capacity(line.len());
    let mut i = 0usize;
    while i < line.len() {
        let rest = &line[i..];
        let rest_bytes = rest.as_bytes();
        // Byte-level prefix match: markers are ASCII, so comparing the leading
        // bytes is char-boundary-safe (a str slice `rest[..m.len()]` would
        // panic if `m.len()` landed inside a multi-byte char like '—').
        let hit = MARKERS.iter().copied().find(|m| {
            rest_bytes.len() >= m.len() && rest_bytes[..m.len()].eq_ignore_ascii_case(m.as_bytes())
        });
        if let Some(m) = hit {
            out.push_str("[REDACTED]");
            i += m.len();
            let tail = &line[i..];
            let consume = if CONSUME_TO_EOL
                .iter()
                .copied()
                .any(|c| m.eq_ignore_ascii_case(c))
            {
                // The whole remainder is the credential (may contain spaces,
                // `=`, `;`). Consume to end-of-line so nothing leaks past a
                // space or delimiter.
                tail.len()
            } else {
                // Single token: alnum + `_`, `-`, `.`. Stop at the first
                // non-token char so the next marker in the same line can
                // still be caught.
                tail.find(|c: char| {
                    !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
                })
                .unwrap_or(tail.len())
            };
            i += consume;
        } else {
            // Advance by one char (UTF-8 safe): copy the whole codepoint.
            let ch = line[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Install the crash-safe panic hook.
///
/// On panic the hook: disables raw mode, leaves the alt screen, restores the
/// cursor, restores the captured audio format (so a mid-loop crash can't strand
/// the DAC at a switched sample rate), then chains to the previous hook so the
/// panic message still prints normally. Every step is best-effort — a panic
/// inside the hook must not itself panic (which would abort).
fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Terminal restore first so the panic message lands on the user's
        // normal screen, not inside the alt screen.
        let _ = cleanup_terminal();
        cleanup_audio();
        // Chain to the previous hook (default: print the panic to stderr).
        prev(info);
    }));
}

/// Register the SIGTSTP / SIGCONT flag handlers. Best-effort: if registration
/// fails (e.g. unsupported platform), the loop just won't honor suspend — it
/// never blocks the core event loop.
fn install_signal_handlers() -> Result<()> {
    use signal_hook::consts::{SIGCONT, SIGTSTP};
    unsafe {
        signal_hook::low_level::register(SIGTSTP, || {
            SIGTSTP_RECEIVED.store(true, Ordering::SeqCst);
        })
        .context("registering SIGTSTP handler")?;
        signal_hook::low_level::register(SIGCONT, || {
            NEED_REDRAW.store(true, Ordering::SeqCst);
        })
        .context("registering SIGCONT handler")?;
    }
    Ok(())
}

/// Suspend the process: restore the terminal, restore audio, set SIGTSTP to its
/// default disposition, raise it (the shell suspends us), and on resume restore
/// our handler + raw/alt-screen mode. Best-effort.
#[cfg(unix)]
fn handle_sigtstp() -> Result<()> {
    use signal_hook::consts::SIGTSTP;
    // Restore terminal + audio so the shell prompt is usable while we're suspended.
    let _ = cleanup_terminal();
    cleanup_audio();

    // Reset SIGTSTP to default so `raise` actually suspends us.
    unsafe {
        let _ = libc::signal(libc::SIGTSTP, libc::SIG_DFL);
        libc::raise(libc::SIGTSTP);
    }

    // --- resumed here on SIGCONT ---
    // Re-arm our SIGTSTP handler + re-enter alt screen / raw mode.
    unsafe {
        signal_hook::low_level::register(SIGTSTP, || {
            SIGTSTP_RECEIVED.store(true, Ordering::SeqCst);
        })?;
    }
    enter_alt_screen()?;
    enable_raw_mode().context("re-enabling raw mode after SIGCONT")?;
    Ok(())
}

#[cfg(not(unix))]
fn handle_sigtstp() -> Result<()> {
    Ok(())
}

/// Enter alt screen + enable mouse capture + show nothing for the cursor
/// (the TUI manages its own cursor). Used both at startup and after SIGCONT.
fn enter_alt_screen() -> Result<()> {
    let mut stdout = std::io::stdout();
    queue!(stdout, EnterAlternateScreen)?;
    queue!(stdout, crossterm::event::EnableMouseCapture)?;
    execute!(stdout, crossterm::cursor::Hide)?;
    Ok(())
}

/// True when the terminal supports raw mode + alt screen (ANSI controls).
/// False for `TERM=dumb`, which can't render ANSI controls or handle raw
/// mode. When false, [`run`] prints a clear message and exits without
/// emitting any ANSI controls — the app refuses to enter raw mode rather
/// than dumping raw escape sequences a dumb terminal can't render (RB-6).
///
/// `TERM` unset → assume raw-capable (the common case for most terminal
/// emulators). An empty `TERM` is treated as dumb (some environments set
/// `TERM=""` to indicate no terminal capabilities).
pub fn terminal_supports_raw_mode() -> bool {
    match std::env::var("TERM") {
        Ok(term) => term != "dumb" && !term.is_empty(),
        Err(_) => true,
    }
}

/// The terminal event loop.
///
/// `captured` is the pre-loop CoreAudio format snapshot (Task 7) used to
/// restore the device on exit/crash. `None` on non-macOS.
pub fn run(app: &mut App, captured: Option<CapturedFormat>) -> Result<()> {
    // Stash the captured format in the global slot so the panic hook can reach
    // it. `get_or_init` because the hook may fire before the loop body runs.
    let captured_slot = CAPTURED.get_or_init(|| Mutex::new(None));
    // If `captured` is Some, drop it into the slot (overwriting any prior None).
    if let Some(fmt) = captured {
        if let Ok(mut guard) = captured_slot.lock() {
            *guard = Some(fmt);
        }
    }

    install_panic_hook();
    install_signal_handlers()?;

    // RB-6: TERM=dumb terminals can't handle raw mode / alt-screen / ANSI
    // controls. Refuse to enter raw mode with a clear message instead of
    // dumping raw escape sequences a dumb terminal can't render.
    if !terminal_supports_raw_mode() {
        let term = std::env::var("TERM").unwrap_or_default();
        eprintln!(
            "jukebox: this terminal (TERM={term}) does not support the raw mode / \
             alternate screen the TUI requires."
        );
        eprintln!(
            "Set TERM=xterm (or similar) and try again, or use a terminal that \
             supports ANSI controls."
        );
        return Ok(());
    }

    let mut stdout = std::io::stdout();
    enable_raw_mode().context("enabling raw mode")?;
    enter_alt_screen()?;
    // Flush the queued enter-alt-screen + mouse-capture commands.
    stdout.flush().context("flushing terminal setup")?;

    // Guard must live from the moment raw + alt-screen are both enabled, so any
    // `?`-return below (Terminal::new, clear, first draw) still restores the
    // terminal via the guard's Drop. Constructed BEFORE `Terminal::new`.
    let _guard = TerminalGuard;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("building ratatui terminal")?;
    terminal.clear()?;

    // Force an initial draw so the screen isn't blank for the first poll tick.
    terminal.draw(|f| view::layout::draw(f, app))?;

    // Last yt_error value we logged, so the loop only writes a log line when
    // the error CHANGES (not every tick). Lives across iterations; reset to
    // None would re-log a persistent error ~6.7×/s.
    let mut last_logged_error: Option<String> = None;

    while !app.should_quit {
        // Drain the SIGCONT redraw flag.
        if NEED_REDRAW.swap(false, Ordering::SeqCst) {
            terminal.clear()?;
            terminal.draw(|f| view::layout::draw(f, app))?;
        }

        // Drain the SIGTSTP suspend request.
        if SIGTSTP_RECEIVED.swap(false, Ordering::SeqCst) {
            handle_sigtstp()?;
            // After resume, the backend's stdout is back in alt screen; force a
            // full redraw on the next iteration.
            terminal.clear()?;
            terminal.draw(|f| view::layout::draw(f, app))?;
            continue;
        }

        // Auto-advance when the player reports a natural end-of-track.
        if app.player.track_ended() {
            app.on_track_ended();
        }

        // Per-tick housekeeping: drain async sidecar responses (Y-view refresh,
        // pre-resolved stream URLs) + auto-restart a crashed sidecar. Without
        // this, fire-and-forget fetches never land and the Y view stays on
        // "loading…" forever (spec §3.1/§3.5).
        app.on_tick();

        // File logging (Slice 7): when `on_tick` surfaces a NEW yt_error, write
        // a redacted line to the log so post-mortem debugging doesn't require a
        // replay. Only logs on CHANGE (compared against `last_logged_error`)
        // so a persistent error doesn't spam one line per tick (~6.7/s).
        if let Some(e) = &app.yt_error {
            if last_logged_error.as_deref() != Some(e.as_str()) {
                log_to_file(&redact(&format!("yt_error: {e}")));
                last_logged_error = Some(e.clone());
            }
        }

        terminal.draw(|f| view::layout::draw(f, app))?;

        if event::poll(std::time::Duration::from_millis(POLL_TIMEOUT_MS))? {
            match event::read()? {
                // macOS crossterm emits both Press and Release events for
                // each keypress. Without this filter, every key fires
                // twice — e.g. `Ctrl+W, e` would: (1) enter edit mode on
                // the Press, then (2) fall through to `enqueue_selected()`
                // on the Release, making it look like `e` didn't work.
                // Only process Press events.
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    handle_key_event(app, k);
                }
                Event::Mouse(m) => input::handle_mouse(app, m),
                Event::Resize(_, _) => {
                    // ratatui auto-handles via Terminal::draw on the next loop
                    // iteration; nothing to do here.
                }
                _ => {}
            }
        }
    }

    // Normal exit: the guard's Drop restores the terminal + audio. Take the
    // captured format back out of the global slot first so the Drop's
    // `restore_output_format` actually runs with a value (the Drop also calls
    // cleanup_audio, which is a no-op if we already took it — but taking it
    // here makes the success-path restore explicit and testable).
    cleanup_audio();

    Ok(())
}

/// Wrap a `KeyEvent` so that keypad/shift ambiguities are normalized before
/// dispatch. Currently a thin pass-through; kept as a seam so future
/// disambiguation (e.g. treating release events) lives in one place.
fn handle_key_event(app: &mut App, k: KeyEvent) {
    input::handle_key(app, k);
}
