//! Key + mouse dispatch: translate crossterm input into [`App`] state changes.
//!
//! [`handle_key`] is the keyboard controller. It is pure with respect to the
//! terminal — it never reads from or writes to the TTY — so the dispatch
//! layer is exhaustively unit-testable without a terminal (see
//! `tests/input.rs`).
//!
//! The keymap follows the design spec (`specs/2026-07-06-tui-revamp-design.md`
//! §Keymap): vim + lazygit/yazi/helix conventions. When an [`Overlay`] is open,
//! keys route to the overlay (typing into search/command, `n`/`N` next/prev
//! match, `Enter` pick, `Esc` close); `Esc` always closes any overlay first.
//!
//! Reserved (never bound): `Ctrl+C`, `Ctrl+Z`, `Ctrl+\`, `Ctrl+S`, `Ctrl+Q`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

use crate::tui::app::{App, Overlay, View};

// ---------------------------------------------------------------------------
// Key dispatch
// ---------------------------------------------------------------------------

/// The single keyboard entry point. Translates `key` into [`App`] state
/// changes per the design-spec keymap.
pub fn handle_key(app: &mut App, key: KeyEvent) {
    // Reserved terminal shortcuts: never bind. (We deliberately ignore them so
    // the terminal's own SIGINT/SIGTSTP/SIGQUIT/XON/XOFF handling stays intact
    // under raw mode.)
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') | KeyCode::Char('z') | KeyCode::Char('\\')
            | KeyCode::Char('s') | KeyCode::Char('q') => return,
            _ => {}
        }
    }

    // Overlay-open routing: typing, n/N, Enter pick, Esc close.
    if app.overlay.is_some() {
        handle_overlay_key(app, key);
        return;
    }

    // Leader-key (`gg`) handling: a pending `g` arms a top-of-column jump; a
    // second `g` consumes it. Any other key cancels the pending state and
    // falls through to normal dispatch.
    let was_pending_g = app.pending_g;
    if !matches!(key.code, KeyCode::Char('g')) {
        app.pending_g = false;
    }

    match (key.code, key.modifiers) {
        // --- Navigation -----------------------------------------------------
        (KeyCode::Char('h'), m) if m == KeyModifiers::NONE => move_left(app),
        (KeyCode::Char('l'), m) if m == KeyModifiers::NONE => move_right(app),
        (KeyCode::Char('j'), m) if m == KeyModifiers::NONE => move_down(app),
        (KeyCode::Char('k'), m) if m == KeyModifiers::NONE => move_up(app),
        (KeyCode::Left, m) if m == KeyModifiers::NONE => move_left(app),
        (KeyCode::Right, m) if m == KeyModifiers::NONE => move_right(app),
        (KeyCode::Down, m) if m == KeyModifiers::NONE => move_down(app),
        (KeyCode::Up, m) if m == KeyModifiers::NONE => move_up(app),

        // `gg` top of column; first `g` arms, second `g` jumps.
        (KeyCode::Char('g'), _) => {
            if was_pending_g {
                top_of_column(app);
                app.pending_g = false;
            } else {
                app.pending_g = true;
            }
        }
        // `G` bottom of column.
        (KeyCode::Char('G'), _) => bottom_of_column(app),

        // View switching: 1/2/3 = Artists/Playlists/Queue.
        (KeyCode::Char('1'), m) if m == KeyModifiers::NONE => switch_view(app, View::Artists),
        (KeyCode::Char('2'), m) if m == KeyModifiers::NONE => switch_view(app, View::Playlists),
        (KeyCode::Char('3'), m) if m == KeyModifiers::NONE => switch_view(app, View::Queue),

        // Tab / Shift+Tab cycle view forward / backward.
        (KeyCode::Tab, m) if m.contains(KeyModifiers::SHIFT) => cycle_view(app, false),
        (KeyCode::Tab, _) => cycle_view(app, true),

        // --- Playback -------------------------------------------------------
        (KeyCode::Enter, _) => app.play_selected(),
        (KeyCode::Char(' '), _) => { let _ = app.player.play_pause(); }
        (KeyCode::Char('>'), _) => app.next(),
        (KeyCode::Char('<'), _) => app.prev(),
        (KeyCode::Char(','), _) => { let _ = app.player.seek(-5.0); }
        (KeyCode::Char('.'), _) => { let _ = app.player.seek(5.0); }
        (KeyCode::Char('+'), _) => app.volume_up(),
        (KeyCode::Char('-'), _) => app.volume_down(),
        (KeyCode::Char('m'), _) => app.toggle_mute(),

        // --- Modes ----------------------------------------------------------
        (KeyCode::Char('z'), _) => app.cycle_shuffle(),
        (KeyCode::Char('Z'), _) => app.reshuffle(),
        (KeyCode::Char('r'), _) => app.cycle_repeat(),

        // --- Overlays -------------------------------------------------------
        (KeyCode::Char('/'), _) => {
            app.overlay = Some(Overlay::Search {
                input: String::new(),
                results: Vec::new(),
                cursor: 0,
            });
        }
        (KeyCode::Char('?'), _) => {
            app.overlay = Some(Overlay::Help);
        }
        (KeyCode::Char('a'), _) => {
            app.overlay = Some(Overlay::PlaylistPicker);
        }
        (KeyCode::Char(':'), _) => {
            app.overlay = Some(Overlay::Command { input: String::new() });
        }

        // --- Quit -----------------------------------------------------------
        (KeyCode::Char('q'), _) => app.quit(),

        _ => {}
    }
}

/// Keys that route to the active overlay. `Esc` closes any overlay; otherwise
/// the key is interpreted by the overlay type (search/command input, `n`/`N`
/// next/prev match, `Enter` pick).
fn handle_overlay_key(app: &mut App, key: KeyEvent) {
    // Esc closes any overlay, first, before anything else.
    if matches!(key.code, KeyCode::Esc) {
        app.overlay = None;
        return;
    }

    match app.overlay.take() {
        Some(Overlay::Search { mut input, results, mut cursor }) => {
            match key.code {
                KeyCode::Char(c) if key.modifiers == KeyModifiers::NONE => input.push(c),
                KeyCode::Backspace => { input.pop(); }
                KeyCode::Enter => {
                    // Play the result under the search cursor in context.
                    if let Some(id) = results.get(cursor).cloned() {
                        app.play_in_context_ids(results.clone(), &id);
                        // play_in_context_ids moved `results`; we're done.
                        return;
                    }
                }
                KeyCode::Char('n') if !results.is_empty() => {
                    cursor = (cursor + 1) % results.len();
                }
                KeyCode::Char('N') if !results.is_empty() => {
                    cursor = cursor.checked_sub(1).unwrap_or(results.len().saturating_sub(1));
                }
                _ => {}
            }
            app.overlay = Some(Overlay::Search { input, results, cursor });
        }
        Some(Overlay::Command { mut input }) => {
            match key.code {
                KeyCode::Char(c) if key.modifiers == KeyModifiers::NONE => input.push(c),
                KeyCode::Backspace => { input.pop(); }
                KeyCode::Enter => {
                    // Command execution is best-effort + minimal for Task 11;
                    // the command parser is wired up in a later task. For now
                    // we just close the command line on submit.
                    app.overlay = None;
                    return;
                }
                _ => {}
            }
            app.overlay = Some(Overlay::Command { input });
        }
        // Help / PlaylistPicker: any non-Esc key is a no-op (overlay stays open
        // until Esc). PlaylistPicker selection routing is wired up in a later
        // task; for now the picker is display-only.
        Some(other) => {
            app.overlay = Some(other);
        }
        None => {}
    }
}

// ---------------------------------------------------------------------------
// Cursor / view navigation helpers
// ---------------------------------------------------------------------------

/// Number of rows in the focused column of the current view.
fn focused_column_len(app: &App) -> usize {
    match app.view {
        View::Artists => match app.focus_col {
            0 => app.artists.len(),
            1 => app
                .artists
                .get(app.cursors.artist)
                .and_then(|a| app.albums_by_artist.get(a))
                .map(|v| v.len())
                .unwrap_or(0),
            _ => focused_track_count(app),
        },
        View::Playlists => match app.focus_col {
            0 => app.playlists.len(),
            _ => focused_track_count(app),
        },
        View::Queue => app.transport.manual_queue.len(),
    }
}

/// Track-list length of the album/playlist currently in view.
fn focused_track_count(app: &App) -> usize {
    match app.view {
        View::Artists => {
            let artist = app.artists.get(app.cursors.artist).cloned().unwrap_or_default();
            app.albums_by_artist
                .get(&artist)
                .and_then(|v| v.get(app.cursors.album))
                .map(|a| a.track_indices.len())
                .unwrap_or(0)
        }
        View::Playlists => app
            .playlists
            .get(app.cursors.playlist)
            .map(|p| p.track_ids.len())
            .unwrap_or(0),
        View::Queue => app.transport.manual_queue.len(),
    }
}

/// Max focus_col index for the current view (Artists=2, Playlists=1, Queue=0).
fn max_focus_col(app: &App) -> usize {
    match app.view {
        View::Artists => 2,
        View::Playlists => 1,
        View::Queue => 0,
    }
}

fn move_left(app: &mut App) {
    if app.focus_col > 0 {
        app.focus_col -= 1;
    }
}

fn move_right(app: &mut App) {
    let max = max_focus_col(app);
    if app.focus_col < max {
        app.focus_col += 1;
    }
}

fn move_up(app: &mut App) {
    let len = focused_column_len(app);
    if len == 0 {
        return;
    }
    let cur = focused_cursor(app);
    if cur > 0 {
        set_focused_cursor(app, cur - 1);
    }
}

fn move_down(app: &mut App) {
    let len = focused_column_len(app);
    if len == 0 {
        return;
    }
    let cur = focused_cursor(app);
    if cur + 1 < len {
        set_focused_cursor(app, cur + 1);
    }
}

fn top_of_column(app: &mut App) {
    set_focused_cursor(app, 0);
}

fn bottom_of_column(app: &mut App) {
    let len = focused_column_len(app);
    set_focused_cursor(app, len.saturating_sub(1));
}

/// The cursor value of the focused column.
fn focused_cursor(app: &App) -> usize {
    match app.view {
        View::Artists => match app.focus_col {
            0 => app.cursors.artist,
            1 => app.cursors.album,
            _ => app.cursors.track,
        },
        View::Playlists => match app.focus_col {
            0 => app.cursors.playlist,
            _ => app.cursors.track,
        },
        View::Queue => app.cursors.queue,
    }
}

fn set_focused_cursor(app: &mut App, v: usize) {
    match app.view {
        View::Artists => match app.focus_col {
            0 => app.cursors.artist = v,
            1 => app.cursors.album = v,
            _ => app.cursors.track = v,
        },
        View::Playlists => match app.focus_col {
            0 => app.cursors.playlist = v,
            _ => app.cursors.track = v,
        },
        View::Queue => app.cursors.queue = v,
    }
}

fn switch_view(app: &mut App, view: View) {
    app.view = view;
    app.focus_col = 0;
}

/// Cycle the browse view forward (`fwd=true`, Tab) or backward (Shift+Tab).
fn cycle_view(app: &mut App, fwd: bool) {
    let next = match (app.view, fwd) {
        (View::Artists, true) => View::Playlists,
        (View::Playlists, true) => View::Queue,
        (View::Queue, true) => View::Artists,
        (View::Artists, false) => View::Queue,
        (View::Playlists, false) => View::Artists,
        (View::Queue, false) => View::Playlists,
    };
    switch_view(app, next);
}

// ---------------------------------------------------------------------------
// Mouse dispatch
// ---------------------------------------------------------------------------

/// Translate a crossterm mouse event into [`App`] state changes.
///
/// Best-effort hit-testing against the rendered layout:
/// - Click in the player bar's transport row → prev / play-pause / next.
/// - Click/drag on the progress gauge → seek proportional to the click x.
/// - Click/drag on the volume meter → set volume proportional to the click x.
/// - Click a browse row → focus + select that row (mapped by approximate
///   column geometry from `app.column_widths`).
/// - Wheel up/down → scroll the focused column.
///
/// The terminal's exact cell geometry isn't known to the controller (only the
/// renderer computes it), so we use the same `column_widths` the renderer uses
/// to map a click `(column, row)` back to a column + rough row index. This is
/// approximate by design — pixel-perfect hit-testing belongs to the view layer
/// in a future refactor.
pub fn handle_mouse(app: &mut App, m: MouseEvent) {
    match m.kind {
        MouseEventKind::ScrollUp => move_up(app),
        MouseEventKind::ScrollDown => move_down(app),
        MouseEventKind::Down(_) | MouseEventKind::Drag(_) => {
            // The player bar occupies the bottom 2 rows. Hits there are
            // transport / progress / volume; hits above are browse.
            let area_height: u16 = 24; // floor; ratatui reports the real size on draw
            let bar_top = area_height.saturating_sub(2);
            if m.row >= bar_top {
                handle_player_bar_click(app, m.column, m.row - bar_top);
            } else {
                handle_browse_click(app, m.column, m.row);
            }
        }
        _ => {}
    }
}

/// Click in the bottom 2-row player bar. Row 0 = info line (transport glyphs
/// ◀◀ ▶ ▶▶ + volume meter); row 1 = progress gauge.
fn handle_player_bar_click(app: &mut App, col: u16, row_in_bar: u16) {
    // We don't know the terminal width here without the renderer, so we use a
    // coarse thirds partitioning of the transport region (which sits near the
    // left of the info line) and a proportional seek/volume for the rest.
    // This is best-effort; precise hit-testing is a view-layer concern.
    if row_in_bar == 1 {
        // Progress gauge row: seek proportional to column / assumed width 80.
        let pct = (col as f64 / 80.0).clamp(0.0, 1.0);
        if let Some(dur) = app.player.duration() {
            if dur > 0.0 {
                let _ = app.player.seek(pct * dur - app.player.position().unwrap_or(0.0));
            }
        }
        return;
    }
    // Row 0: transport glyphs live in roughly columns 18..32 (◀◀ ▶ ▶▶). Clicks
    // in that band map to prev / play-pause / next; clicks further right set
    // volume proportionally.
    if (18..=32).contains(&col) {
        match col {
            18..=21 => app.prev(),
            22..=27 => { let _ = app.player.play_pause(); }
            _ => app.next(),
        }
        return;
    }
    // Otherwise treat as a volume meter click: proportional set. The volume
    // meter sits in roughly the right third of the info line.
    let pct = (col as f64 / 80.0).clamp(0.0, 1.0);
    app.volume = (pct * 100.0).round() as u8;
    app.muted = false;
}

/// Click in the browse area: map `col` to a focus column using `column_widths`
/// and `row` to a rough row index in that column.
fn handle_browse_click(app: &mut App, col: u16, row: u16) {
    let cw = &app.column_widths;
    let rail = cw.rail;
    if col < rail {
        // Clicked the rail: switch view by row.
        match row {
            0 => switch_view(app, View::Artists),
            1 => switch_view(app, View::Playlists),
            2 => switch_view(app, View::Queue),
            _ => {}
        }
        return;
    }
    let col_no_rail = col - rail;
    // Determine which focus column the click landed in based on cumulative
    // widths. Subtract 1 for the header row.
    let focus = if col_no_rail < cw.col1 {
        0
    } else if col_no_rail < (cw.col1 + cw.col2) {
        match app.view {
            View::Artists => 1,
            _ => 1,
        }
    } else {
        match app.view {
            View::Artists => 2,
            _ => 1,
        }
    };
    app.focus_col = focus;
    let row_index = row.saturating_sub(1) as usize; // -1 for the column header
    let len = focused_column_len_with_focus(app, focus);
    if len > 0 {
        let clamped = row_index.min(len - 1);
        set_focused_cursor_with_focus(app, focus, clamped);
    }
}

/// Variant of [`focused_column_len`] for an explicit `focus` (used during click
/// hit-testing before we've committed the focus change).
fn focused_column_len_with_focus(app: &App, focus: usize) -> usize {
    // Mirror of [`focused_column_len`] for an explicit `focus` (used during
    // click hit-testing before we've committed the focus change).
    match app.view {
        View::Artists => match focus {
            0 => app.artists.len(),
            1 => app
                .artists
                .get(app.cursors.artist)
                .and_then(|a| app.albums_by_artist.get(a))
                .map(|v| v.len())
                .unwrap_or(0),
            _ => focused_track_count(app),
        },
        View::Playlists => match focus {
            0 => app.playlists.len(),
            _ => focused_track_count(app),
        },
        View::Queue => app.transport.manual_queue.len(),
    }
}

fn set_focused_cursor_with_focus(app: &mut App, focus: usize, v: usize) {
    app.focus_col = focus;
    set_focused_cursor(app, v);
}
