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

    // Inline filter (`f`) routing: typing narrows the focused column, Esc/Enter
    // clear. Navigation keys (arrows) fall through to normal dispatch and
    // operate on the filtered list.
    if app.filter.is_some() && handle_filter_key(app, key) {
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

        // View switching: 1/2/3/4 = Artists/Playlists/Queue/YouTube.
        (KeyCode::Char('1'), m) if m == KeyModifiers::NONE => switch_view(app, View::Artists),
        (KeyCode::Char('2'), m) if m == KeyModifiers::NONE => switch_view(app, View::Playlists),
        (KeyCode::Char('3'), m) if m == KeyModifiers::NONE => switch_view(app, View::Queue),
        (KeyCode::Char('4'), m) if m == KeyModifiers::NONE => switch_view(app, View::Youtube),

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
        // `c` cycles continue mode (mode-dependent: see App::cycle_continue).
        (KeyCode::Char('c'), _) => app.cycle_continue(),
        // `M` cycles the source mode Local → YouTube → Mixed → Local (never
        // stops playback).
        (KeyCode::Char('M'), _) => app.cycle_mode(),
        // `s` instant random track in context; `S` discover overlay (spec §5.5).
        (KeyCode::Char('s'), _) => app.instant_random(),
        (KeyCode::Char('S'), _) => app.open_discover(),

        // --- Overlays -------------------------------------------------------
        (KeyCode::Char('/'), _) => {
            app.overlay = Some(Overlay::Search {
                input: String::new(),
                results: Vec::new(),
                cursor: 0,
            });
        }
        // `f` inline filter on the focused column (spec §5.4).
        (KeyCode::Char('f'), _) => app.toggle_filter(),
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
        Some(Overlay::Search { mut input, mut results, mut cursor }) => {
            // Track whether the query changed so we only re-search on mutation.
            let mut changed = false;
            match key.code {
                // Enter picks the result under the cursor (play in context).
                KeyCode::Enter => {
                    if let Some(id) = results.get(cursor).cloned() {
                        let ids = std::mem::take(&mut results);
                        app.play_in_context_ids(ids, &id);
                        return;
                    }
                }
                // Arrow keys are the ONLY result navigators in the search
                // overlay. Letters (including `n`, `j`, `k`) are never bound
                // here — they always go into the query so you can search for
                // anything ("nirvana", "joji", …) without a key being
                // swallowed as navigation.
                KeyCode::Down if !results.is_empty() => {
                    cursor = (cursor + 1) % results.len();
                }
                KeyCode::Up if !results.is_empty() => {
                    cursor = cursor.checked_sub(1).unwrap_or(results.len().saturating_sub(1));
                }
                // Accept Char regardless of SHIFT so capital letters (and
                // shifted symbols) make it into the input — a Shift+F would
                // otherwise be dropped because its modifiers != NONE.
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    input.push(c);
                    changed = true;
                }
                KeyCode::Backspace => {
                    input.pop();
                    changed = true;
                }
                _ => {}
            }
            app.overlay = Some(Overlay::Search { input, results, cursor });
            if changed {
                app.update_search_results();
            }
        }
        Some(Overlay::Command { mut input }) => {
            match key.code {
                // Accept Char regardless of SHIFT — see the Search arm note.
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) => input.push(c),
                KeyCode::Backspace => { input.pop(); }
                KeyCode::Enter => {
                    let cmd = input.trim().to_string();
                    app.overlay = None;
                    execute_command(app, &cmd);
                    return;
                }
                _ => {}
            }
            app.overlay = Some(Overlay::Command { input });
        }
        Some(Overlay::YtAuth { mut input }) => {
            // The auth overlay's own keymap: typing accumulates the pasted
            // cookies; `Enter` saves+connects; `Esc` cancels (handled above).
            // Pasted newlines arrive as Char('\n') — push them as spaces so the
            // whole cookies.txt stays on one logical line (Netscape format is
            // tab-delimited; joining with spaces still parses).
            match key.code {
                KeyCode::Enter => {
                    app.apply_yt_auth(std::mem::take(&mut input));
                    app.overlay = None;
                    return;
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    input.push(if c == '\n' || c == '\r' { ' ' } else { c });
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                _ => {}
            }
            app.overlay = Some(Overlay::YtAuth { input });
        }
        // Help / PlaylistPicker: any non-Esc key is a no-op (overlay stays open
        // until Esc). PlaylistPicker selection routing is wired up in a later
        // task; for now the picker is display-only.
        Some(Overlay::Discover { items, mut cursor }) => {
            match key.code {
                KeyCode::Esc => {
                    app.overlay = None;
                    return;
                }
                KeyCode::Down if !items.is_empty() => {
                    cursor = (cursor + 1) % items.len();
                }
                KeyCode::Up if !items.is_empty() => {
                    cursor = cursor.checked_sub(1).unwrap_or(items.len().saturating_sub(1));
                }
                KeyCode::Enter => {
                    app.overlay = Some(Overlay::Discover { items, cursor });
                    app.play_discover_selection();
                    return;
                }
                _ => {}
            }
            app.overlay = Some(Overlay::Discover { items, cursor });
        }
        Some(other) => {
            app.overlay = Some(other);
        }
        None => {}
    }
}

/// Keys that route to the active inline filter (`f`). Returns true if the key
/// was consumed (Char/Backspace/Esc/Enter); false for navigation keys, which
/// fall through to normal dispatch and operate on the filtered list.
fn handle_filter_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            app.filter = None;
            true
        }
        KeyCode::Enter => {
            // Enter on a filter jumps the cursor to the first match + clears.
            app.filter_jump();
            true
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT) =>
        {
            if let Some(f) = &mut app.filter {
                f.text.push(c);
            }
            true
        }
        KeyCode::Backspace => {
            if let Some(f) = &mut app.filter {
                f.text.pop();
            }
            true
        }
        _ => false, // navigation keys fall through
    }
}

/// Execute a `:` command. v1 supports `:yt auth`, `:yt logout`, `:yt setup`.
fn execute_command(app: &mut App, cmd: &str) {
    match cmd {
        "yt auth" => {
            app.overlay = Some(Overlay::YtAuth { input: String::new() });
        }
        "yt logout" => {
            app.yt_logout();
        }
        "yt setup" => {
            app.yt_error = Some(
                "install deps: pip install -r scripts/yt/requirements.txt".into(),
            );
        }
        _ => {}
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
        View::Youtube => match app.focus_col {
            0 => app.yt_lists.len(),
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
                .map(|a| app.tracks_for_album(&a.title).len())
                .unwrap_or(0)
        }
        View::Playlists => app
            .playlists
            .get(app.cursors.playlist)
            .map(|p| p.track_ids.len())
            .unwrap_or(0),
        View::Youtube => app
            .yt_lists
            .get(app.cursors.playlist)
            .map(|l| l.track_ids.len())
            .unwrap_or(0),
        View::Queue => app.transport.manual_queue.len(),
    }
}

/// Max focus_col index for the current view (Artists=2, Playlists=1, Queue=0).
fn max_focus_col(app: &App) -> usize {
    match app.view {
        View::Artists => 2,
        View::Playlists => 1,
        View::Youtube => 1,
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
        View::Youtube => match app.focus_col {
            0 => app.cursors.playlist,
            _ => app.cursors.track,
        },
        View::Queue => app.cursors.queue,
    }
}

fn set_focused_cursor(app: &mut App, v: usize) {
    match app.view {
        View::Artists => match app.focus_col {
            // Changing the artist invalidates the album + track cursors: the
            // new artist has a different album list, so the old album/track
            // indices point at the wrong thing (or past the end → empty
            // Tracks column, the "this artist has no songs" bug). Reset them.
            0 => {
                app.cursors.artist = v;
                app.cursors.album = 0;
                app.cursors.track = 0;
            }
            // Changing the album invalidates the track cursor (different track
            // list). Reset it so Enter plays a valid track.
            1 => {
                app.cursors.album = v;
                app.cursors.track = 0;
            }
            _ => app.cursors.track = v,
        },
        View::Playlists => match app.focus_col {
            // Changing the playlist invalidates the track cursor.
            0 => {
                app.cursors.playlist = v;
                app.cursors.track = 0;
            }
            _ => app.cursors.track = v,
        },
        View::Youtube => match app.focus_col {
            // Changing the YT list invalidates the track cursor + triggers a
            // lazy fetch of its tracks (wired in Task 13).
            0 => {
                app.cursors.playlist = v;
                app.cursors.track = 0;
            }
            _ => app.cursors.track = v,
        },
        View::Queue => app.cursors.queue = v,
    }
}

fn switch_view(app: &mut App, view: View) {
    app.view = view;
    app.focus_col = 0;
    // Entering the Y view fetches the account + suggested lists (bounded
    // synchronous roundtrip at the view-enter boundary; spec §5.3).
    if view == View::Youtube {
        app.refresh_yt_lists();
    }
}

/// Cycle the browse view forward (`fwd=true`, Tab) or backward (Shift+Tab).
fn cycle_view(app: &mut App, fwd: bool) {
    let next = match (app.view, fwd) {
        (View::Artists, true) => View::Playlists,
        (View::Playlists, true) => View::Queue,
        (View::Queue, true) => View::Youtube,
        (View::Youtube, true) => View::Artists,
        (View::Artists, false) => View::Youtube,
        (View::Youtube, false) => View::Queue,
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
        MouseEventKind::Down(_) => {
            // A single click — route to the player bar (transport/seek) or the
            // browse columns. Drag is deliberately NOT routed: a held-drag used
            // to scrub volume on every mouse-move, which jumped the level
            // erratically. Volume is keyboard-only (+/-/m) now.
            let (_, h) = crossterm::terminal::size().unwrap_or((80, 24));
            // Player bar is 2 rows above the 1-row footer hint bar, so its top
            // is at h-3; the footer (row h-1) is intentionally not clickable.
            let bar_top = h.saturating_sub(3);
            let footer_row = h.saturating_sub(1);
            if m.row >= bar_top && m.row < footer_row {
                handle_player_bar_click(app, m.column, m.row - bar_top);
            } else if m.row < bar_top {
                handle_browse_click(app, m.column, m.row);
            }
            // clicks on the footer row are ignored
        }
        _ => {}
    }
}

/// Click in the bottom 2-row player bar. Row 0 is the info line (now-playing
/// text, transport glyphs, volume, mode flags); row 1 is the progress gauge.
/// Only the transport glyphs and the progress gauge are clickable. The
/// now-playing text and volume meter are intentionally not clickable, because
/// coarse hit-testing there made a stray click jump volume to an arbitrary
/// value.
fn handle_player_bar_click(app: &mut App, col: u16, row_in_bar: u16) {
    let width = crossterm::terminal::size().map(|(w, _)| w).unwrap_or(80).max(1);
    if row_in_bar == 1 {
        // Progress gauge row: click-to-seek, proportional to column / width.
        let pct = (col as f64 / width as f64).clamp(0.0, 1.0);
        if let Some(dur) = app.player.duration() {
            if dur > 0.0 {
                let _ = app.player.seek(pct * dur - app.player.position().unwrap_or(0.0));
            }
        }
        return;
    }
    // Row 0: only the transport glyphs (◀◀ ▶ ▶▶, roughly cols 18..32) are
    // clickable. Anything else (now-playing text, volume, flags) is ignored.
    if (18..=32).contains(&col) {
        match col {
            18..=21 => app.prev(),
            22..=27 => { let _ = app.player.play_pause(); }
            _ => app.next(),
        }
    }
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
            3 => switch_view(app, View::Youtube),
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
        View::Youtube => match focus {
            0 => app.yt_lists.len(),
            _ => focused_track_count(app),
        },
        View::Queue => app.transport.manual_queue.len(),
    }
}

fn set_focused_cursor_with_focus(app: &mut App, focus: usize, v: usize) {
    app.focus_col = focus;
    set_focused_cursor(app, v);
}
