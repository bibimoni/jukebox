//! Phase 2 tests: overlay backgrounds + diagnostics border.
//!
//! Verifies that the 10 previously-hardcoded `bg(Color::Black)` overlay
//! backdrop sites now route through `Theme::overlay()`, which returns
//! `Reset` (not `Black`) under `NO_COLOR=1` (visual spec C4 / A2).
//! Also verifies the diagnostics overlay now has an accent-colored
//! border + title (visual spec H17 / V16).

use jukebox::tui::view::theme::Theme;
use ratatui::style::Color;
use std::sync::{Mutex, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Lock the env mutex, recovering from poison so a single test panic
/// doesn't cascade failures to every other env-mutating test in the file.
fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    env_lock().lock().unwrap_or_else(|e| e.into_inner())
}

/// Collect a single row of cell symbols from the buffer (joined into one
/// string) so a multi-char message can be searched as a substring.
fn row_str(buf: &ratatui::buffer::Buffer, y: u16, width: u16) -> String {
    (0..width).map(|x| buf[(x, y)].symbol()).collect()
}

#[test]
fn theme_overlay_returns_black_in_color_mode() {
    let _guard = lock_env();
    std::env::remove_var("NO_COLOR");
    let t = Theme::default();
    assert_eq!(
        t.overlay().bg,
        Some(Color::Black),
        "color-mode overlay backdrop is Black (visible modal on dark terminal)"
    );
}

#[test]
fn theme_overlay_returns_reset_under_no_color() {
    let _guard = lock_env();
    std::env::set_var("NO_COLOR", "1");
    let t = Theme::default();
    assert_eq!(
        t.overlay().bg,
        Some(Color::Reset),
        "NO_COLOR overlay backdrop must be Reset (no-color.org, no \\e[40m)"
    );
    std::env::remove_var("NO_COLOR");
}

#[test]
fn theme_overlay_returns_black_in_high_contrast() {
    let _guard = lock_env();
    std::env::remove_var("NO_COLOR");
    std::env::set_var("JUKEBOX_HIGH_CONTRAST", "1");
    let t = Theme::default();
    // high_contrast assumes a dark terminal (documented in Help); the
    // backdrop stays Black so the modal is visible.
    assert_eq!(
        t.overlay().bg,
        Some(Color::Black),
        "high-contrast overlay backdrop is Black (modal visibility on dark terminal)"
    );
    std::env::remove_var("JUKEBOX_HIGH_CONTRAST");
}

/// Smoke-test: the diagnostics overlay renders without panic at the
/// minimum size and the border has the accent color. Phase 2 H17/V16.
#[test]
fn diagnostics_overlay_renders_with_accent_border() {
    use jukebox::diagnostics::Diagnostics;
    use jukebox::tui::view::diagnostics::render;
    use ratatui::{backend::TestBackend, layout::Rect, Terminal};

    let _guard = lock_env();
    std::env::remove_var("NO_COLOR");
    let diag = Diagnostics::new();
    let backend = TestBackend::new(40, 10);
    let mut term = Terminal::new(backend).unwrap();
    let area = Rect::new(0, 0, 40, 10);
    term.draw(|f| render(f, area, &diag)).unwrap();
    let buf = term.backend().buffer();

    // The four corner cells should be border glyphs styled with the
    // accent color (visual spec H17 / V16: diagnostics border must
    // match every other overlay's accent color, not the terminal default).
    let theme = Theme::default();
    let corner = &buf[(0, 0)];
    assert!(
        !corner.symbol().is_empty(),
        "diagnostics border corner must render"
    );
    assert_eq!(
        corner.style().fg,
        Some(theme.accent),
        "diagnostics border must be accent-colored (was default-colored pre-Phase 2)"
    );

    // The title cell (top row, after the corner) should also be accent
    // (via Theme::status_key which is accent + BOLD).
    let title_cell = &buf[(2, 0)];
    assert_eq!(
        title_cell.style().fg,
        Some(theme.accent),
        "diagnostics title must be accent-colored"
    );
}

/// The diagnostics overlay renders without panic at tiny sizes.
#[test]
fn diagnostics_overlay_no_panic_at_tiny_sizes() {
    use jukebox::diagnostics::Diagnostics;
    use jukebox::tui::view::diagnostics::render;
    use ratatui::{backend::TestBackend, layout::Rect, Terminal};

    let _guard = lock_env();
    std::env::remove_var("NO_COLOR");
    let diag = Diagnostics::new();
    for &(w, h) in &[(1, 1), (5, 1), (10, 3), (40, 10), (80, 24)] {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, w, h);
        // No panic at any size — saturating_sub everywhere.
        term.draw(|f| render(f, area, &diag)).unwrap();
    }
}

/// The diagnostics overlay shows the placeholder when empty and the
/// "(newest first)" header + messages when populated. Phase 2 doesn't
/// change this behavior, but it's a useful regression guard.
#[test]
fn diagnostics_overlay_renders_messages_newest_first() {
    use jukebox::diagnostics::Diagnostics;
    use jukebox::tui::view::diagnostics::render;
    use ratatui::{backend::TestBackend, layout::Rect, Terminal};

    let _guard = lock_env();
    std::env::remove_var("NO_COLOR");

    // Empty buffer → placeholder.
    let diag = Diagnostics::new();
    let backend = TestBackend::new(40, 10);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| render(f, Rect::new(0, 0, 40, 10), &diag))
        .unwrap();
    let buf = term.backend().buffer();
    let row1 = row_str(buf, 1, 40);
    assert!(
        row1.contains("no diagnostics yet"),
        "empty diagnostics should show placeholder, got: {row1}"
    );

    // Populated buffer → "(newest first)" header + messages, newest first.
    let mut diag = Diagnostics::new();
    diag.push("oldest message".to_string());
    diag.push("middle message".to_string());
    diag.push("newest message".to_string());
    let backend = TestBackend::new(40, 10);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| render(f, Rect::new(0, 0, 40, 10), &diag))
        .unwrap();
    let buf = term.backend().buffer();

    // Build per-row strings so we can search for multi-char messages.
    let rows: Vec<String> = (0..10).map(|y| row_str(buf, y, 40)).collect();

    // The "(newest first)" header + every message must be present.
    let has_header = rows.iter().any(|r| r.contains("newest first"));
    assert!(
        has_header,
        "header '(newest first)' must be present: {rows:?}"
    );
    let has_newest = rows.iter().any(|r| r.contains("newest message"));
    let has_oldest = rows.iter().any(|r| r.contains("oldest message"));
    assert!(has_newest, "newest message must be rendered");
    assert!(has_oldest, "oldest message must be rendered");

    // The newest message must be ABOVE the oldest message (newest-first).
    let newest_y = rows
        .iter()
        .position(|r| r.contains("newest message"))
        .unwrap_or(usize::MAX);
    let oldest_y = rows
        .iter()
        .position(|r| r.contains("oldest message"))
        .unwrap_or(usize::MAX);
    assert!(
        newest_y < oldest_y,
        "newest message (y={newest_y}) must be above oldest (y={oldest_y})"
    );
}
