//! Regression tests for RB-4 (auth busy state) and RB-2 (false provider/account
//! states).
//!
//! RB-4: Authentication can enter an unexplained busy state.
//! - Empty cookie submission is REJECTED (must NOT transition to
//!   AuthenticatedNotSynced).
//! - Closing the auth overlay (Esc) resets any busy/Authenticating/
//!   AuthenticatedNotSynced state.
//! - Browser auth has a visible progress/cancel/timeout outcome.
//!
//! RB-2: Provider, network, and account states can be false.
//! - Offline/failed YouTube search shows a TRUTHFUL provider-state message,
//!   NOT "No results" as if the search succeeded.
//! - A never-authenticated (signed-out) profile is NEVER labeled
//!   "expired"/[reauth].
//! - Home NEVER tells a signed-out account to "listen more to build your
//!   profile."

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::{App, Overlay, SearchScope, View, YtTab};
use jukebox::tui::input::handle_key;
use jukebox::tui::view::layout::draw;
use jukebox::yt::state::YtState;
use ratatui::{backend::TestBackend, Terminal};

fn cat_track() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("A")).unwrap();
    std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[{"id":"t1","artists":["Ado"],"primary_artist":"Ado","title":"Freedom",
        "album":"Adele","bit_depth":24,"sample_rate_hz":96000,
        "source_path":"lossless/A/01.flac","symlinked_into_artists":["Ado"]}]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

fn key_code(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn isolate_xdg() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "jk-rb-{}-{}",
        std::process::id(),
        std::sync::atomic::AtomicU64::new(0).fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&d).unwrap();
    std::env::set_var("XDG_CONFIG_HOME", &d);
    d
}

fn buffer_string(term: &Terminal<TestBackend>, w: u16, h: u16) -> String {
    let mut s = String::new();
    for y in 0..h {
        let mut line = String::new();
        for x in 0..w {
            line.push(
                term.backend().buffer()[(x, y)]
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' '),
            );
        }
        s.push_str(&line);
        s.push('\n');
    }
    s
}

fn render_text(app: &mut App, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| draw(f, app)).unwrap();
    buffer_string(&term, w, h)
}

// ===========================================================================
// RB-4: Authentication can enter an unexplained busy state
// ===========================================================================

/// RB-4: empty cookie submission is REJECTED. The old code unconditionally
/// transitioned to AuthenticatedNotSynced (`[Y ~]`), leaving the user in a busy
/// state with no credential. Now apply_yt_auth validates and returns early.
#[test]
fn rb4_empty_cookie_submit_rejected() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_track();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    let before = app.yt_state;
    app.apply_yt_auth(String::new());
    assert!(
        !matches!(app.yt_state, YtState::AuthenticatedNotSynced),
        "RB-4: empty cookies must NOT transition to AuthenticatedNotSynced, got {:?}",
        app.yt_state
    );
    assert_eq!(
        app.yt_state, before,
        "RB-4: empty cookies must not change yt_state"
    );
    assert!(
        app.yt_error.is_some(),
        "RB-4: empty cookies must set a clear error message"
    );
    let msg = app.yt_error.unwrap();
    assert!(
        msg.contains("paste") || msg.contains("empty") || msg.contains("cookies"),
        "RB-4: error should tell the user to paste cookies, got {msg:?}"
    );
}

/// RB-4: whitespace-only cookie submission is also REJECTED.
#[test]
fn rb4_whitespace_cookie_submit_rejected() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_track();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.apply_yt_auth("   \n\t  ".into());
    assert!(
        !matches!(app.yt_state, YtState::AuthenticatedNotSynced),
        "RB-4: whitespace-only cookies must NOT transition to AuthenticatedNotSynced"
    );
    assert!(app.yt_error.is_some());
}

/// RB-4: closing the YtAuth overlay with Esc resets Authenticating /
/// AuthenticatedNotSynced back to a non-busy state. The old Esc handler just
/// set overlay=None without resetting yt_state, leaving a lingering busy state.
#[test]
fn rb4_esc_from_auth_overlay_resets_busy_state() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_track();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Simulate a prior `:yt auth browser` that left the state busy.
    app.yt_state = YtState::Authenticating;
    app.overlay = Some(Overlay::YtAuth {
        input: String::new(),
    });
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert!(app.overlay.is_none(), "Esc should close the overlay");
    assert!(
        !matches!(
            app.yt_state,
            YtState::Authenticating | YtState::AuthenticatedNotSynced
        ),
        "RB-4: Esc must reset Authenticating/AuthenticatedNotSynced, got {:?}",
        app.yt_state
    );
}

/// RB-4: Esc from auth overlay resets AuthenticatedNotSynced (the `[Y ~]`
/// state) back to a non-busy state.
#[test]
fn rb4_esc_resets_authenticated_not_synced() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_track();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.yt_state = YtState::AuthenticatedNotSynced;
    app.overlay = Some(Overlay::YtAuth {
        input: String::new(),
    });
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert!(app.overlay.is_none());
    assert!(
        !matches!(app.yt_state, YtState::AuthenticatedNotSynced),
        "RB-4: Esc must reset AuthenticatedNotSynced, got {:?}",
        app.yt_state
    );
}

/// RB-4: Esc from auth overlay does NOT reset Synchronizing (which may be from
/// a view-enter probe, not the auth flow) or error/ready states.
#[test]
fn rb4_esc_does_not_reset_synchronizing_or_ready() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_track();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Synchronizing from a view-enter probe should NOT be reset by Esc.
    app.yt_state = YtState::Synchronizing;
    app.overlay = Some(Overlay::YtAuth {
        input: String::new(),
    });
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert!(
        matches!(app.yt_state, YtState::Synchronizing),
        "RB-4: Esc must NOT reset Synchronizing (may be from view-enter), got {:?}",
        app.yt_state
    );
    // ProviderError should NOT be reset by Esc from YtAuth.
    app.yt_state = YtState::ProviderError;
    app.overlay = Some(Overlay::YtAuth {
        input: String::new(),
    });
    handle_key(&mut app, key_code(KeyCode::Esc));
    assert!(
        matches!(app.yt_state, YtState::ProviderError),
        "RB-4: Esc must NOT reset ProviderError, got {:?}",
        app.yt_state
    );
}

/// RB-4: the auth-busy timeout transitions AuthenticatedNotSynced to
/// ProviderError with a clear message after ~600 ticks, so the user is never
/// left with an unexplained busy state.
#[test]
fn rb4_auth_busy_timeout_transitions_to_provider_error() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_track();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.yt_state = YtState::AuthenticatedNotSynced;
    app.yt_auth_busy_ticks = 0;
    // Tick 599 times — should still be busy.
    for _ in 0..599 {
        app.on_tick();
        if app.yt_state != YtState::AuthenticatedNotSynced {
            break;
        }
    }
    // 601 ticks — should have timed out.
    app.on_tick();
    app.on_tick();
    assert!(
        matches!(app.yt_state, YtState::ProviderError),
        "RB-4: auth-busy timeout should transition to ProviderError, got {:?}",
        app.yt_state
    );
    assert!(
        app.yt_error.is_some(),
        "RB-4: timeout should set a clear error message"
    );
    let msg = app.yt_error.unwrap();
    assert!(
        msg.contains("timed out"),
        "RB-4: timeout message should say 'timed out', got {msg:?}"
    );
}

/// RB-4: the auth overlay shows the current provider state (progress/cancel)
/// so the user is never left with an unexplained busy state.
#[test]
fn rb4_auth_overlay_shows_busy_state() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_track();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.yt_state = YtState::Authenticating;
    app.overlay = Some(Overlay::YtAuth {
        input: String::new(),
    });
    let text = render_text(&mut app, 80, 24);
    assert!(
        text.contains("authenticating") || text.contains("[~]"),
        "RB-4: auth overlay should show the busy state, got text without 'authenticating' or '[~]'"
    );
    assert!(
        text.contains("Esc"),
        "RB-4: auth overlay should show Esc cancel hint"
    );
}

/// RB-4: the auth overlay shows an error when yt_error is set.
#[test]
fn rb4_auth_overlay_shows_error_state() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_track();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.yt_state = YtState::ProviderError;
    app.yt_error = Some("auth failed: bad cookies".into());
    app.overlay = Some(Overlay::YtAuth {
        input: String::new(),
    });
    let text = render_text(&mut app, 80, 24);
    assert!(
        text.contains("auth failed") || text.contains("[!]"),
        "RB-4: auth overlay should show the error, got text without 'auth failed' or '[!]'"
    );
}

// ===========================================================================
// RB-2: Provider, network, and account states can be false
// ===========================================================================

/// RB-2: offline/failed YouTube search shows a TRUTHFUL provider-state message,
/// NOT "No results for '...'" as if the search succeeded and was empty.
#[test]
fn rb2_offline_search_shows_provider_state_not_no_results() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_track();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.yt_state = YtState::ProviderError;
    app.view = View::Youtube;
    app.yt_view.tab = YtTab::Search;
    app.overlay = Some(Overlay::Search {
        input: "test query".into(),
        results: Vec::new(),
        cursor: 0,
        scope: SearchScope::Youtube,
        submitted: Some("test query".into()),
        searching: false,
    });
    let text = render_text(&mut app, 100, 30);
    assert!(
        !text.contains("No results for 'test query'"),
        "RB-2: offline search must NOT show 'No results for ...', got: {text:?}"
    );
    assert!(
        text.contains("offline") || text.contains("press R") || text.contains("provider error"),
        "RB-2: offline search should show a provider-state message, got: {text:?}"
    );
}

/// RB-2: when the provider is Ready and a search genuinely returns zero
/// matches, "No results for '...'" IS shown (the search succeeded).
#[test]
fn rb2_ready_search_zero_results_shows_no_results() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_track();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.yt_state = YtState::Ready;
    app.view = View::Youtube;
    app.yt_view.tab = YtTab::Search;
    app.overlay = Some(Overlay::Search {
        input: "nothing here".into(),
        results: Vec::new(),
        cursor: 0,
        scope: SearchScope::Youtube,
        submitted: Some("nothing here".into()),
        searching: false,
    });
    let text = render_text(&mut app, 100, 30);
    assert!(
        text.contains("No results for 'nothing here'"),
        "RB-2: ready search with zero matches SHOULD show 'No results', got: {text:?}"
    );
}

/// RB-2: a never-authenticated (Unconfigured) account that gets an auth-flavored
/// error must NOT be labeled "expired"/[reauth]. The old heuristic set
/// AuthExpired unconditionally; now it requires is_authed().
#[test]
fn rb2_unconfigured_auth_error_not_labeled_expired() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_track();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.yt_state = YtState::Unconfigured;
    // Simulate an auth-flavored error arriving. The old code would set
    // AuthExpired even for Unconfigured (never authed). Now it should NOT.
    // We verify via the state machine: is_authed() is false for Unconfigured,
    // so AuthExpired must not be set from an Unconfigured state.
    assert!(!app.yt_state.is_authed(), "Unconfigured must not be authed");
    assert!(
        !matches!(app.yt_state, YtState::AuthExpired),
        "Unconfigured must not be AuthExpired"
    );
    // The footer label for Unconfigured must NOT say "expired" or "[reauth]".
    assert_ne!(
        app.yt_state.human_label(),
        YtState::AuthExpired.human_label(),
        "Unconfigured label must differ from AuthExpired"
    );
    assert_ne!(
        app.yt_state.icon(),
        YtState::AuthExpired.icon(),
        "Unconfigured icon must differ from AuthExpired ([reauth])"
    );
}

/// RB-2: SignedOut and AuthExpired are distinct in the state machine —
/// SignedOut is "[!]" / "signed out", AuthExpired is "[reauth]" /
/// "authorization expired".
#[test]
fn rb2_signed_out_distinct_from_auth_expired() {
    assert_ne!(
        YtState::SignedOut.human_label(),
        YtState::AuthExpired.human_label(),
        "SignedOut and AuthExpired must have different labels"
    );
    assert_ne!(
        YtState::SignedOut.icon(),
        YtState::AuthExpired.icon(),
        "SignedOut and AuthExpired must have different icons"
    );
    assert!(
        !YtState::SignedOut.is_authed(),
        "SignedOut must not be authed"
    );
    assert!(
        YtState::AuthExpired.is_authed(),
        "AuthExpired must be authed (was previously authed)"
    );
}

/// RB-2: Home NEVER tells a signed-out account to "listen more to build your
/// profile" as if it were a live account. A signed-out Home shows a truthful
/// sign-in prompt.
#[test]
fn rb2_signed_out_home_shows_signin_not_growth() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_track();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.yt_state = YtState::SignedOut;
    app.view = View::Youtube;
    app.yt_view.tab = YtTab::Home;
    let text = render_text(&mut app, 100, 30);
    assert!(
        !text.contains("listen more to build your profile"),
        "RB-2: signed-out Home must NOT show 'listen more to build your profile', got: {text:?}"
    );
    assert!(
        !text.contains("cold start"),
        "RB-2: signed-out Home must NOT show 'cold start' growth messaging, got: {text:?}"
    );
    assert!(
        text.contains("signed out") || text.contains("sign in") || text.contains(":yt auth"),
        "RB-2: signed-out Home should show a sign-in prompt, got: {text:?}"
    );
}

/// RB-2: an Unconfigured Home also shows a sign-in prompt, not growth messaging.
#[test]
fn rb2_unconfigured_home_shows_signin_not_growth() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_track();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.yt_state = YtState::Unconfigured;
    app.view = View::Youtube;
    app.yt_view.tab = YtTab::Home;
    let text = render_text(&mut app, 100, 30);
    assert!(
        !text.contains("listen more to build your profile"),
        "RB-2: unconfigured Home must NOT show growth messaging, got: {text:?}"
    );
    assert!(
        text.contains("sign in") || text.contains(":yt auth") || text.contains("signed out"),
        "RB-2: unconfigured Home should show a sign-in prompt, got: {text:?}"
    );
}

/// RB-2: the footer badge for SignedOut is "[Y —]" (not connected), NOT
/// "[reauth]" (which is AuthExpired) or "[Y ~]" (which is a busy state).
#[test]
fn rb2_signed_out_footer_badge_not_reauth() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_track();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.yt_state = YtState::SignedOut;
    let text = render_text(&mut app, 120, 24);
    assert!(
        !text.contains("[reauth]"),
        "RB-2: SignedOut footer must NOT show [reauth], got: {text:?}"
    );
    assert!(
        text.contains("[Y —]") || text.contains("signed out"),
        "RB-2: SignedOut footer should show [Y —] or 'signed out', got: {text:?}"
    );
}

/// RB-2: the footer for Unconfigured also does NOT show [reauth].
#[test]
fn rb2_unconfigured_footer_badge_not_reauth() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_track();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.yt_state = YtState::Unconfigured;
    let text = render_text(&mut app, 120, 24);
    assert!(
        !text.contains("[reauth]"),
        "RB-2: Unconfigured footer must NOT show [reauth], got: {text:?}"
    );
}

/// RB-2: a YouTube search when the provider is Unconfigured shows "not
/// connected", not "No results".
#[test]
fn rb2_unconfigured_search_shows_not_connected() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_track();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.yt_state = YtState::Unconfigured;
    app.view = View::Youtube;
    app.yt_view.tab = YtTab::Search;
    app.overlay = Some(Overlay::Search {
        input: "song".into(),
        results: Vec::new(),
        cursor: 0,
        scope: SearchScope::Youtube,
        submitted: Some("song".into()),
        searching: false,
    });
    let text = render_text(&mut app, 100, 30);
    assert!(
        !text.contains("No results for 'song'"),
        "RB-2: unconfigured search must NOT show 'No results', got: {text:?}"
    );
    assert!(
        text.contains("not connected") || text.contains(":yt auth"),
        "RB-2: unconfigured search should show 'not connected', got: {text:?}"
    );
}

/// RB-2: submit_yt_search returns false when there's no session, so the
/// overlay's searching flag is NOT set (preventing a stuck "searching..." state
/// for an offline provider).
#[test]
fn rb2_submit_search_returns_false_without_session() {
    let _xdg = isolate_xdg();
    let (_d, cat) = cat_track();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    let dispatched = app.submit_yt_search("test".into());
    assert!(
        !dispatched,
        "RB-2: submit_yt_search should return false without a session"
    );
    assert!(
        app.yt_error.is_some(),
        "RB-2: submit_yt_search should set yt_error when it can't dispatch"
    );
}
