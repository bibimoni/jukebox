//! Slice 1 tests: the truthful YouTube provider state machine.
//!
//! Verifies the core invariants fixed by S1.2–S1.6:
//! - A failed data fetch keeps the session (no `yt_session = None`), only the
//!   state label changes — the fix for the "repeatedly must log in" symptom.
//! - `Ready` is reached ONLY after a successful data fetch, never before —
//!   the fix for the "connected in logs but playlists empty" symptom.
//! - Auth-flavored errors demote to `AuthExpired`; other errors to
//!   `ProviderError`.
//! - `yt_logout` clears cached lists and transitions to `SignedOut`.
//! - The footer status text never says "connected" for a pre-fetch state.
//!
//! Uses the same fake-sidecar pattern as `tests/e2e_yt.rs`: a per-test Python
//! script that echoes canned JSON lines keyed by `cmd`.

use jukebox::tui::app::App;
use jukebox::yt::session::Session;
use jukebox::yt::state::YtState;
use std::io::Write;

/// Write a per-test fake sidecar script + its canned-response map file. The
/// map path is baked into the script (no env var) so parallel tests can't race.
/// Mirrors `e2e_yt::fake_sidecar`.
#[allow(clippy::write_literal)]
fn fake_sidecar(map_json: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let map = std::env::temp_dir().join(format!("ps-map-{}-{}.json", std::process::id(), n));
    std::fs::write(&map, map_json).unwrap();
    let p = std::env::temp_dir().join(format!("ps-fake-{}-{}.py", std::process::id(), n));
    let mut f = std::fs::File::create(&p).unwrap();
    write!(
        f,
        "import sys, json\nm = json.load(open({map_path:?}))\n",
        map_path = map.display(),
    )
    .unwrap();
    f.write_all(
        r#"
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try: req = json.loads(line)
    except Exception: continue
    cmd = req.get("cmd")
    key = m.get(cmd)
    if cmd == "resolve_url":
        vid = req.get("video_id", "")
        key = json.dumps({"ok": True, "data": {"resolve": {"url": "https://x/" + vid, "expires_at": None, "codec": "AAC", "abr": 256, "sample_rate": 48000, "container": "m4a", "premium": True}}})
    if key is not None:
        print(key, flush=True)
"#
        .as_bytes(),
    )
    .unwrap();
    writeln!(f).unwrap();
    (p, map)
}

fn local_cat() -> (tempfile::TempDir, jukebox::catalog::Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("Adele")).unwrap();
    std::fs::write(lossless.join("Adele").join("01.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[{"id":"t1","artists":["Adele"],"primary_artist":"Adele","title":"Hello",
        "album":"25","bit_depth":24,"sample_rate_hz":96000,"source_path":"lossless/Adele/01.flac",
        "symlinked_into_artists":["Adele"],"isrc":"GBBKS1500123"}]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, jukebox::catalog::Catalog::load(&p).unwrap())
}

/// Pump `on_tick` (with small sleeps for the sidecar reader thread to deliver
/// responses) until `cond(app)` is true, up to `max` iterations.
fn tick_until<F>(app: &mut App, max: usize, cond: F) -> bool
where
    F: Fn(&App) -> bool,
{
    for _ in 0..max {
        app.on_tick();
        if cond(&*app) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    false
}

/// A successful refresh must promote `yt_state` to `Ready` — the ONLY promotion
/// point from a pre-fetch state to a ready state. This is the fix for the
/// "connected but empty" bug: "connected" (now `Ready`) cannot appear before a
/// data fetch has actually succeeded.
#[test]
fn refresh_success_promotes_to_ready() {
    let map = r#"{"library_playlists":"{\"ok\":true,\"data\":{\"playlists\":[{\"id\":\"PL1\",\"name\":\"Liked\",\"count\":3}]}}","home_suggestions":"{\"ok\":true,\"data\":{\"suggestions\":[]}}"}"#;
    let (script, map_file) = fake_sidecar(map);
    let session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = jukebox::tui::app::View::Youtube;
    // Before refresh: default state is Unconfigured (no fetch yet).
    assert_eq!(app.yt_state, YtState::Unconfigured);
    app.refresh_yt_lists();
    assert!(
        app.yt_state.is_transient(),
        "refresh sets a transient state"
    );
    // Pump until on_tick folds the playlists + promotes to Ready.
    let promoted = tick_until(&mut app, 100, |a| a.yt_state == YtState::Ready);
    assert!(
        promoted,
        "on_tick should promote to Ready after a successful fetch"
    );
    assert!(app.yt_state.is_ready());
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

/// A failed refresh must NOT discard the session — only the state label
/// changes to `ProviderError`. This is the fix for the "repeatedly must log
/// in" root cause: the old blocking probe set `yt_session = None` on any error,
/// discarding the session (and the Keychain-cached cookies) for the whole run.
#[test]
fn refresh_error_keeps_session_and_demotes_to_provider_error() {
    // "connection timed out" has no auth keywords → ProviderError (not AuthExpired).
    let map = r#"{"library_playlists":"{\"ok\":false,\"error\":\"connection timed out\"}","home_suggestions":"{\"ok\":true,\"data\":{\"suggestions\":[]}}"}"#;
    let (script, map_file) = fake_sidecar(map);
    let session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = jukebox::tui::app::View::Youtube;
    app.refresh_yt_lists();
    // Pump until on_tick drains the error + demotes the state.
    let demoted = tick_until(&mut app, 100, |a| a.yt_state == YtState::ProviderError);
    assert!(
        demoted,
        "on_tick should demote to ProviderError on a non-auth error"
    );
    // The session MUST still be alive — this is the core fix.
    assert!(
        app.yt_session.is_some(),
        "a failed fetch must NOT discard the session (the repeated-login root cause)"
    );
    assert!(!app.yt_state.is_ready(), "ProviderError is not ready");
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

/// An auth-flavored error (containing "401"/"unauthorized") must demote to
/// `AuthExpired` (needs re-auth), NOT `ProviderError` (needs retry). The
/// recovery hint differs: "run :yt auth browser" vs "press R to retry".
#[test]
fn auth_error_demotes_to_auth_expired() {
    let map = r#"{"library_playlists":"{\"ok\":false,\"error\":\"HTTP 401 Unauthorized\"}","home_suggestions":"{\"ok\":true,\"data\":{\"suggestions\":[]}}"}"#;
    let (script, map_file) = fake_sidecar(map);
    let session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.refresh_yt_lists();
    let demoted = tick_until(&mut app, 100, |a| a.yt_state == YtState::AuthExpired);
    assert!(
        demoted,
        "a 401/unauthorized error should demote to AuthExpired, not ProviderError"
    );
    assert!(app.yt_session.is_some(), "session kept on auth expiry");
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

/// `yt_logout` must clear cached YT lists (so stale data doesn't survive
/// logout) and transition to `SignedOut` (distinct from `Unconfigured`: the
/// user took an explicit action).
#[test]
fn logout_clears_lists_and_sets_signed_out() {
    let map = r#"{"library_playlists":"{\"ok\":true,\"data\":{\"playlists\":[{\"id\":\"PL1\",\"name\":\"Liked\",\"count\":3}]}}","home_suggestions":"{\"ok\":true,\"data\":{\"suggestions\":[]}}"}"#;
    let (script, map_file) = fake_sidecar(map);
    let session = Session::spawn(std::path::Path::new("python3"), &script, None).unwrap();
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        Some(session),
    );
    app.view = jukebox::tui::app::View::Youtube;
    app.refresh_yt_lists();
    // Wait for lists to populate.
    tick_until(&mut app, 100, |a| !a.yt_lists.is_empty());
    assert!(!app.yt_lists.is_empty(), "lists populated before logout");
    app.loaded_yt_lists.insert("PL1".into());
    assert!(!app.loaded_yt_lists.is_empty());
    // Logout.
    app.yt_logout();
    assert_eq!(app.yt_state, YtState::SignedOut, "logout → SignedOut");
    assert!(app.yt_lists.is_empty(), "logout must clear yt_lists");
    assert!(
        app.loaded_yt_lists.is_empty(),
        "logout must clear loaded_yt_lists"
    );
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&map_file);
}

/// The false-ready invariant: `AuthenticatedNotSynced` (we have credentials but
/// no data fetch has verified them) must be authed but NOT ready. The footer
/// and Y-view status derive from this enum, so "connected" can no longer appear
/// before data is verified. This is the core state-machine unit test.
#[test]
fn no_false_connected_authenticated_not_synced_is_not_ready() {
    assert!(YtState::AuthenticatedNotSynced.is_authed());
    assert!(!YtState::AuthenticatedNotSynced.is_ready());
    assert!(!YtState::Synchronizing.is_ready());
    assert!(!YtState::Authenticating.is_ready());
    assert!(YtState::Ready.is_ready());
    assert!(YtState::ReadyStale.is_ready());
}

/// The footer status text must NOT contain "connected" when the provider is in
/// a pre-fetch state (`AuthenticatedNotSynced`). The old code set
/// `yt_status = "connected via chrome"` here (yt-recon §8), which was false-
/// ready. Now `yt_status_text()` derives from `yt_state.human_label()`, which
/// says "authenticated — syncing…" — never "connected".
#[test]
fn footer_status_text_never_says_connected_before_fetch() {
    let (_d, cat) = local_cat();
    let mut app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        None,
    );
    // Simulate the post-auth, pre-fetch state (what `:yt auth browser` sets
    // before the launch refresh lands).
    app.yt_state = YtState::AuthenticatedNotSynced;
    let text = app
        .yt_status_text()
        .expect("non-ready state should produce status text");
    assert!(
        !text.to_lowercase().contains("connected"),
        "footer must not say 'connected' before a data fetch verifies the credential: {text}"
    );
    // Ready with no transient message → None (the hint line shows).
    app.yt_state = YtState::Ready;
    assert!(
        app.yt_status_text().is_none(),
        "Ready + no yt_status → hint line"
    );
    // Ready with a transient non-state message (e.g. "upgraded to AAC 256k")
    // still shows that message.
    app.yt_status = Some("upgraded to AAC 256k · YT Premium".into());
    assert_eq!(
        app.yt_status_text().as_deref(),
        Some("upgraded to AAC 256k · YT Premium")
    );
}

/// A no-session app (spawn failed / no python3) must not be ready and must
/// surface a status line (not the bare hint line), so the user knows YT is
/// unavailable from any view.
#[test]
fn no_session_is_not_ready_and_shows_status() {
    let (_d, cat) = local_cat();
    let app = App::new(
        cat,
        Box::new(jukebox::player::StubPlayer::default()),
        None,
        None,
    );
    assert_eq!(app.yt_state, YtState::Unconfigured);
    assert!(!app.yt_state.is_ready());
    assert!(
        app.yt_status_text().is_some(),
        "Unconfigured shows a status line"
    );
}
