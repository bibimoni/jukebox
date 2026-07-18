//! Regression tests for Fixer H defects (Batch H: Help documentation).
//!
//! - H.1 RC11-DEF-005: Unknown command no feedback (persists from RC-01
//!   DEF-003). `input.rs` sets `yt_error` but the footer only rendered it
//!   when `yt_state == Ready`; the default state is `Unconfigured`, so the
//!   error was invisible. Fixed via a generic `status_toast` rendered by the
//!   footer regardless of `yt_state`.
//! - H.2 RC11-DEF-025: No source badge legend in help.
//! - H.3 RC11-DEF-047: Empty `:` + Tab no command list.
//! - H.4 Help accuracy pass: resume key, radio c/q/>, generator s, publish
//!   Tab, Home H, :gen, :radio, :publish documented; stale entries removed.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::{App, Overlay};
use jukebox::tui::input::handle_key;
use jukebox::tui::view::layout::draw;
use jukebox::tui::view::overlay::help_lines;
use ratatui::{backend::TestBackend, Terminal};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn key_code(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn one_track_cat() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("A")).unwrap();
    std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[{"id":"t1","artists":["A"],"primary_artist":"A","title":"Local Song","album":"Al",
        "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/A/01.flac",
        "symlinked_into_artists":["A"]}]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

fn isolate_xdg() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "jk-fixh-{}-{}",
        std::process::id(),
        std::sync::atomic::AtomicU64::new(0).fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&d).unwrap();
    std::env::set_var("XDG_CONFIG_HOME", &d);
    d
}

/// Render the full TUI into a flat string.
fn rendered(app: &mut App, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| draw(f, app)).unwrap();
    let mut buf = String::new();
    for y in 0..h {
        for x in 0..w {
            let c = &term.backend().buffer()[(x, y)];
            buf.push(c.symbol().chars().next().unwrap_or(' '));
        }
        buf.push('\n');
    }
    buf
}

/// Type a `:` command into the app (open command bar, type chars, Enter).
fn type_command(app: &mut App, cmd: &str) {
    handle_key(app, key(':'));
    for c in cmd.chars() {
        handle_key(app, key(c));
    }
    handle_key(app, key_code(KeyCode::Enter));
}

/// Open the command bar WITHOUT submitting (for Tab-completion tests).
fn open_command(app: &mut App) {
    handle_key(app, key(':'));
}

// ---------------------------------------------------------------------------
// H.1 RC11-DEF-005: unknown command feedback visible regardless of yt_state
// ---------------------------------------------------------------------------

/// `:foobar` Enter must surface "unknown command" in the footer status line
/// within one frame, even when `yt_state` is `Unconfigured` (the default at
/// first launch — no YT session). The old footer only rendered `yt_error`
/// when `yt_state == Ready`, so the error was invisible to local-only users.
#[test]
fn def005_unknown_command_visible_in_footer_when_unconfigured() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Default yt_state is Unconfigured — the bug condition.
    assert_eq!(
        app.yt_state,
        jukebox::yt::state::YtState::Unconfigured,
        "fixture: default state must be Unconfigured to reproduce the bug"
    );
    type_command(&mut app, "foobar");
    let buf = rendered(&mut app, 100, 30);
    assert!(
        buf.contains("unknown command"),
        "DEF-005: footer must show 'unknown command' immediately after `:foobar` Enter \
         even when yt_state is Unconfigured: got footer\n{buf}"
    );
}

/// The error toast must NOT require opening the diagnostics overlay (`D`).
/// It must be visible in the standard footer on the very next render.
#[test]
fn def005_unknown_command_visible_without_diagnostics_overlay() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    type_command(&mut app, "foobar");
    // No overlay should be open (command bar closed on Enter).
    assert!(app.overlay.is_none(), "command bar should close on Enter");
    let buf = rendered(&mut app, 100, 30);
    assert!(
        buf.contains("unknown command"),
        "DEF-005: error must be visible without opening diagnostics: got\n{buf}"
    );
}

/// The error must also be captured in `yt_error` for the diagnostics overlay
/// (D key) — the toast is for immediate feedback, yt_error for the log.
#[test]
fn def005_unknown_command_also_recorded_in_yt_error() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    type_command(&mut app, "foobar");
    assert!(
        app.yt_error.is_some(),
        "DEF-005: yt_error should still be set for the diagnostics overlay"
    );
    assert!(
        app.yt_error
            .as_deref()
            .unwrap_or("")
            .contains("unknown command"),
        "DEF-005: yt_error should contain 'unknown command': got {:?}",
        app.yt_error
    );
}

// ---------------------------------------------------------------------------
// H.2 RC11-DEF-025: source badge legend in help
// ---------------------------------------------------------------------------

/// The help overlay must include a "Source badges" section explaining
/// `[L]` local, `[Y]` YouTube, `[Y!]` expired/unavailable.
#[test]
fn def025_help_includes_source_badge_legend() {
    let lines = help_lines(80, false);
    let joined: String = lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("Source badges"),
        "DEF-025: help must have a 'Source badges' section: got\n{joined}"
    );
    assert!(
        joined.contains("[L]"),
        "DEF-025: legend must explain [L] (local): got\n{joined}"
    );
    assert!(
        joined.contains("[Y]"),
        "DEF-025: legend must explain [Y] (YouTube): got\n{joined}"
    );
    assert!(
        joined.contains("[Y!]"),
        "DEF-025: legend must explain [Y!] (expired/unavailable): got\n{joined}"
    );
    // Every badge must have a text label alongside (no emoji, no symbol-only).
    assert!(
        joined.contains("local"),
        "DEF-025: [L] must have 'local' text label"
    );
    assert!(
        joined.contains("YouTube"),
        "DEF-025: [Y] must have 'YouTube' text label"
    );
}

/// The badge legend must also render in ASCII font mode (no Unicode leaks).
#[test]
fn def025_help_badge_legend_in_ascii_mode() {
    let lines = help_lines(80, true);
    let joined: String = lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("Source badges"),
        "DEF-025: ASCII help must have 'Source badges' section: got\n{joined}"
    );
    assert!(
        joined.contains("[L]") && joined.contains("[Y]") && joined.contains("[Y!]"),
        "DEF-025: ASCII legend must include [L] [Y] [Y!]: got\n{joined}"
    );
}

// ---------------------------------------------------------------------------
// H.3 RC11-DEF-047: empty `:` + Tab shows command list
// ---------------------------------------------------------------------------

/// `:` + Tab with no prefix must surface the list of available commands (not
/// a silent no-op). The list appears in the footer status toast so the user
/// can see what commands exist without guessing.
#[test]
fn def047_empty_colon_tab_shows_command_list() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    open_command(&mut app);
    handle_key(&mut app, key_code(KeyCode::Tab));
    // A status toast listing commands must be set.
    assert!(
        app.status_toast.is_some(),
        "DEF-047: `:` + Tab must set a status_toast listing commands: got {:?}",
        app.status_toast
    );
    let toast = app.status_toast.as_deref().unwrap_or("");
    // At least a few known commands must appear in the toast.
    assert!(
        toast.contains("yt") || toast.contains("radio") || toast.contains("queue"),
        "DEF-047: toast must list available commands (yt/radio/queue...): got {toast:?}"
    );
    // The command bar must still be open (Tab doesn't submit).
    assert!(
        matches!(app.overlay, Some(Overlay::Command { .. })),
        "DEF-047: command bar must stay open after Tab (not submit): got {:?}",
        app.overlay
    );
}

/// Prefix-based Tab must still complete a single match (existing behavior
/// must not regress): `:ra` + Tab completes to `radio`.
#[test]
fn def047_prefix_tab_completes_single_match() {
    let _xdg = isolate_xdg();
    let (_d, cat) = one_track_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    open_command(&mut app);
    handle_key(&mut app, key('r'));
    handle_key(&mut app, key('a'));
    handle_key(&mut app, key_code(KeyCode::Tab));
    match &app.overlay {
        Some(Overlay::Command { input, .. }) => {
            assert_eq!(
                *input, "radio",
                "DEF-047: `:ra` + Tab must complete to 'radio': got {input:?}"
            );
        }
        other => panic!("DEF-047: command bar must be open after prefix Tab: got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// H.4 Help accuracy pass
// ---------------------------------------------------------------------------

/// Flatten help_lines into a single string for substring checks.
fn help_text(ascii: bool) -> String {
    help_lines(80, ascii)
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Radio overlay keys `c` (change seed) and `q` (stop) must be documented
/// (Batch B wired them; help must not be stale).
#[test]
fn def_h4_help_documents_radio_c_and_q() {
    let t = help_text(false);
    // The radio section must mention c and q.
    assert!(
        t.contains("c") && t.contains("change seed"),
        "H.4: help must document radio `c` change seed: got\n{t}"
    );
    assert!(
        t.contains("q") && t.contains("stop"),
        "H.4: help must document radio `q` stop: got\n{t}"
    );
}

/// `>` in radio (advance) must be documented (Batch B DEF-026 wired it).
#[test]
fn def_h4_help_documents_radio_gt_advance() {
    let t = help_text(false);
    // The radio section must mention `>` (or `n`/`>` next/advance).
    assert!(
        t.contains(">"),
        "H.4: help radio section must mention `>` (advance): got\n{t}"
    );
}

/// Generator `s` (save alias) must be documented (Batch C DEF-060 wired it).
#[test]
fn def_h4_help_documents_generator_s_save() {
    let t = help_text(false);
    assert!(
        t.contains("s") && (t.contains("save") || t.contains("Save")),
        "H.4: help must document generator `s` save: got\n{t}"
    );
}

/// Publish Tab (cycle privacy) must be documented (Batch C wired it).
#[test]
fn def_h4_help_documents_publish_tab() {
    let t = help_text(false);
    assert!(
        t.contains("Tab") && t.contains("privacy"),
        "H.4: help must document publish Tab cycles privacy: got\n{t}"
    );
}

/// `:gen`, `:radio`, `:publish` commands must all be documented.
#[test]
fn def_h4_help_documents_gen_radio_publish_commands() {
    let t = help_text(false);
    assert!(t.contains(":gen"), "H.4: help must document :gen: got\n{t}");
    assert!(
        t.contains(":radio"),
        "H.4: help must document :radio: got\n{t}"
    );
    assert!(
        t.contains(":publish"),
        "H.4: help must document :publish: got\n{t}"
    );
}

/// Home (`H`) must be documented as the YouTube Home entry.
#[test]
fn def_h4_help_documents_home_key() {
    let t = help_text(false);
    assert!(
        t.contains("H") && t.contains("Home"),
        "H.4: help must document H (YouTube Home): got\n{t}"
    );
}

/// Resume: the help must document the `R` resume key (Batch D wired R to
/// resume the last-played track). The hint text in the player bar must say
/// "R to resume", not the stale "Enter to resume" (Enter plays the selected
/// track, not the resume).
#[test]
fn def_h4_help_documents_resume_key() {
    let t = help_text(false);
    assert!(
        t.contains("R") && (t.contains("resume") || t.contains("Resume")),
        "H.4: help must document R resume: got\n{t}"
    );
}
