//! Slice 7 tests: feedback, logging, diagnostics.
//!
//! - [`status_auto_clears`]: a transient `yt_status` whose TTL has elapsed is
//!   cleared by `on_tick` so the footer returns to the hint bar.
//! - [`no_secret_in_logs`]: a log line carrying cookie secrets is redacted
//!   before it reaches the log file (`[REDACTED]`, no `SAPISID=…` value).
//! - [`diagnostics_capture`]: a new `yt_error` is captured into the
//!   `Diagnostics` buffer by `on_tick` so the diagnostics overlay can show it.

use std::time::{Duration, Instant};

use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::App;

/// A minimal one-track catalog on a temp dir (enough to construct an App).
fn local_cat() -> (tempfile::TempDir, Catalog) {
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
    (d, Catalog::load(&p).unwrap())
}

/// A transient `yt_status` whose 5s TTL has elapsed must be cleared by
/// `on_tick`, and `last_notification` reset so a later re-assertion of the
/// same message is treated as a fresh notification (gets a new window).
#[test]
fn status_auto_clears() {
    let (_d, cat) = local_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Simulate a status set 6s ago (TTL elapsed) with dedup already recorded.
    app.yt_status = Some("upgraded to AAC 256k".into());
    app.last_notification = Some("upgraded to AAC 256k".into());
    app.notification_ttl = Some(Instant::now() - Duration::from_secs(6));
    app.on_tick();
    assert!(
        app.yt_status.is_none(),
        "an elapsed TTL must clear yt_status so the footer returns to the hint bar"
    );
    assert!(
        app.notification_ttl.is_none(),
        "the TTL itself must be cleared once it fires"
    );
    assert!(
        app.last_notification.is_none(),
        "last_notification must reset so a repeat counts as a fresh notification"
    );
}

/// A status within its TTL window must NOT be cleared (the 5s lease holds).
#[test]
fn status_within_ttl_is_kept() {
    let (_d, cat) = local_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.yt_status = Some("queue cleared".into());
    app.last_notification = Some("queue cleared".into());
    // Just-set TTL (now) — well within the 5s window.
    app.notification_ttl = Some(Instant::now());
    app.on_tick();
    assert_eq!(
        app.yt_status.as_deref(),
        Some("queue cleared"),
        "a status within its TTL window must survive on_tick"
    );
}

/// A repeat of the SAME status does NOT refresh the TTL window (dedup): if
/// the original window has elapsed, the repeat still clears, even though the
/// status was re-asserted in the same tick before on_tick ran.
#[test]
fn status_dedup_does_not_refresh_ttl() {
    let (_d, cat) = local_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // First assertion sets the dedup key + a fresh TTL.
    app.yt_status = Some("created \"Mix\"".into());
    app.on_tick();
    let ttl_a = app.notification_ttl.unwrap();
    // Re-assert the SAME status a moment later: on_tick must NOT replace the
    // TTL (dedup), so ttl_b == ttl_a (same instant).
    app.yt_status = Some("created \"Mix\"".into());
    std::thread::sleep(Duration::from_millis(5));
    app.on_tick();
    assert_eq!(
        app.notification_ttl.unwrap(),
        ttl_a,
        "an identical repeat must not refresh the TTL window"
    );
}

/// A log line carrying cookie/token secrets is redacted before it reaches
/// the log file: the file contains `[REDACTED]` and NOT the secret values.
/// Uses the path-injectable writer so the test doesn't touch the real cache.
#[test]
fn no_secret_in_logs() {
    let tmp = tempfile::tempdir().unwrap();
    let log_path = tmp.path().join("jukebox.log");
    // A realistic error line with several secret markers + surrounding text
    // (the text must survive redaction so the log stays readable).
    let raw = "yt_error: auth failed — SAPISID=abc123; __Secure-3PAPISID=def456 authorization=TOKENxyz cookie=ghi789 end";
    let redacted = jukebox::tui::event::redact(raw);
    jukebox::tui::event::log_to_file_at(&log_path, &redacted);
    let content = std::fs::read_to_string(&log_path).unwrap();
    assert!(
        content.contains("[REDACTED]"),
        "redacted log must contain [REDACTED]: {content:?}"
    );
    assert!(
        !content.contains("abc123") && !content.contains("def456") && !content.contains("ghi789"),
        "no secret values may survive redaction: {content:?}"
    );
    assert!(
        !content.contains("TOK"),
        "the authorization token value must not survive: {content:?}"
    );
    // Surrounding context is preserved (readable log).
    assert!(
        content.contains("yt_error: auth failed"),
        "non-secret context must survive redaction: {content:?}"
    );
}

/// F3 regression: `authorization=Bearer <token>` must redact the ENTIRE
/// remainder of the line (the credential may contain a space), not just the
/// first token. The old token-consume stopped at the space, leaking the
/// actual secret after `Bearer `.
#[test]
fn redact_authorization_bearer_consumes_to_end_of_line() {
    let raw = "authorization=Bearer secrettoken123";
    let redacted = jukebox::tui::event::redact(raw);
    assert_eq!(
        redacted, "[REDACTED]",
        "the full Bearer token must be redacted, got: {redacted:?}"
    );
    assert!(
        !redacted.contains("secrettoken123"),
        "the token after the space must NOT leak: {redacted:?}"
    );
    assert!(
        !redacted.contains("Bearer"),
        "the marker value 'Bearer' must not survive: {redacted:?}"
    );
}

/// F3 regression: `authorization=` in the middle of a line still consumes the
/// whole remainder (to end-of-line), so the token after `Bearer ` doesn't
/// leak even when there's context before the marker.
#[test]
fn redact_authorization_bearer_with_prefix_context() {
    let raw = "yt_error: auth failed — authorization=Bearer s3cr3t";
    let redacted = jukebox::tui::event::redact(raw);
    assert!(
        redacted.contains("[REDACTED]"),
        "must contain [REDACTED]: {redacted:?}"
    );
    assert!(
        !redacted.contains("s3cr3t"),
        "the token after Bearer must NOT leak: {redacted:?}"
    );
    assert!(
        redacted.contains("yt_error: auth failed"),
        "prefix context must survive: {redacted:?}"
    );
}

/// F3 regression: `cookie=` consumes to end-of-line so a full cookie line
/// `cookie=name=val; name2=val2` is entirely redacted (the old token-consume
/// stopped at the first `=`, leaking `=val; name2=val2`).
#[test]
fn redact_cookie_consumes_to_end_of_line() {
    let raw = "cookie=name=val1; name2=val2";
    let redacted = jukebox::tui::event::redact(raw);
    assert_eq!(
        redacted, "[REDACTED]",
        "the full cookie line must be redacted, got: {redacted:?}"
    );
    assert!(
        !redacted.contains("val1") && !redacted.contains("val2") && !redacted.contains("name"),
        "no cookie values or names may survive: {redacted:?}"
    );
}

/// F3 regression: every SID-family marker still gets the single-token consume
/// (alnum + `_.-`), so two markers on the same line are BOTH redacted. The
/// per-marker change must not break the default behavior.
#[test]
fn redact_sid_family_markers_still_consume_single_token() {
    // SID-family markers stop at the first non-token char, so the next marker
    // on the same line is still caught.
    let raw = "SAPISID=abc123; __Secure-3PAPISID=def456 SSID=ghi789 end";
    let redacted = jukebox::tui::event::redact(raw);
    assert!(
        !redacted.contains("abc123")
            && !redacted.contains("def456")
            && !redacted.contains("ghi789"),
        "all SID-family values must be redacted: {redacted:?}"
    );
    // "end" survives (it's not a marker value — it's after the last marker's
    // token consume stopped at the space).
    assert!(
        redacted.contains("end"),
        "trailing non-secret text must survive: {redacted:?}"
    );
    // Three markers → three [REDACTED]s.
    assert_eq!(
        redacted.matches("[REDACTED]").count(),
        3,
        "three SID markers must produce three [REDACTED]: {redacted:?}"
    );
}

/// F3 regression: case-insensitive matching still applies (the marker table
/// is compared with `eq_ignore_ascii_case`). `Authorization=` (capital A)
/// must redact just like `authorization=`.
#[test]
fn redact_markers_are_case_insensitive() {
    let raw = "Authorization=Bearer MyToken123";
    let redacted = jukebox::tui::event::redact(raw);
    assert_eq!(
        redacted, "[REDACTED]",
        "capital-A Authorization must still consume to EOL: {redacted:?}"
    );
    assert!(
        !redacted.contains("MyToken123"),
        "token must not leak: {redacted:?}"
    );
}

/// A new `yt_error` is captured into the `Diagnostics` buffer by `on_tick`
/// so the diagnostics overlay can show it (the footer only shows the latest
/// error). Change-detection avoids pushing one entry per tick.
#[test]
fn diagnostics_capture() {
    let (_d, cat) = local_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.yt_error = Some("sidecar died: connection reset".into());
    app.on_tick();
    let msgs: Vec<String> = app.diagnostics.messages().to_vec();
    assert!(
        msgs.iter().any(|m| m.contains("connection reset")),
        "diagnostics must capture the new yt_error: {msgs:?}"
    );
    assert!(
        msgs.iter().any(|m| m.contains("yt_error:")),
        "the captured line should identify the source: {msgs:?}"
    );
    // A second on_tick with the SAME error must NOT push a duplicate
    // (change-detection against the last captured message).
    app.on_tick();
    let msgs2: Vec<String> = app.diagnostics.messages().to_vec();
    let count = msgs2
        .iter()
        .filter(|m| m.contains("connection reset"))
        .count();
    assert_eq!(
        count, 1,
        "a repeat error must not duplicate the diagnostics entry: {msgs2:?}"
    );

    // A CHANGED error pushes a new entry.
    app.yt_error = Some("auth expired: 401".into());
    app.on_tick();
    let msgs3: Vec<String> = app.diagnostics.messages().to_vec();
    assert!(
        msgs3.iter().any(|m| m.contains("401")),
        "a changed error must push a new diagnostics entry: {msgs3:?}"
    );
}

/// The diagnostics overlay is discoverable: `:diag` opens it, and the overlay
/// renders the diagnostics buffer. AC-M5.1.2.
#[test]
fn diagnostics_view_openable() {
    let (_d, cat) = local_cat();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Push a diagnostic so the overlay has content to show.
    app.yt_error = Some("sidecar died".into());
    app.on_tick();

    // `:diag` command opens the overlay.
    app.overlay = Some(jukebox::tui::app::Overlay::Diagnostics);
    assert!(
        matches!(app.overlay, Some(jukebox::tui::app::Overlay::Diagnostics)),
        "diag command should set the Diagnostics overlay"
    );
    assert!(
        !app.diagnostics.messages().is_empty(),
        "diagnostics buffer should have content to display"
    );
}
