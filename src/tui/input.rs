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

use crate::reco::feedback::FeedbackAction;
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
            KeyCode::Char('c')
            | KeyCode::Char('z')
            | KeyCode::Char('\\')
            | KeyCode::Char('s')
            | KeyCode::Char('q') => return,
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
        // `G` opens the playlist generator in the YouTube view (where the
        // recommendation features live); in other views it remains the
        // vim-style "bottom of column" jump so the existing keymap is
        // preserved. Also reachable via `:gen`.
        (KeyCode::Char('G'), _) => {
            if app.view == View::Youtube {
                app.open_generator();
            } else {
                bottom_of_column(app);
            }
        }

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
        (KeyCode::Char(' '), _) => {
            let _ = app.player.play_pause();
        }
        (KeyCode::Char('>'), _) => app.next(),
        (KeyCode::Char('<'), _) => app.prev(),
        (KeyCode::Char(','), _) => {
            let _ = app.player.seek(-5.0);
        }
        (KeyCode::Char('.'), _) => {
            let _ = app.player.seek(5.0);
        }
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
        // `R` retry the YouTube provider probe after an error/stale state
        // (ProviderError / AuthExpired / RateLimited / ReadyStale). No-op when
        // the state is healthy (Ready) or needs auth (Unconfigured/SignedOut) —
        // `retry_yt_probe` guards with `can_retry()`. This is the fix for the
        // "repeated login" root cause: press R instead of re-authenticating.
        (KeyCode::Char('R'), _) => app.retry_yt_probe(),

        // --- Queue & playlist ----------------------------------------------
        // `e` enqueues the focused track to the manual "play next" queue.
        (KeyCode::Char('e'), _) => app.enqueue_selected(),
        // `x` removes the focused track from the manual queue (Queue view).
        (KeyCode::Char('x'), _) => app.remove_selected_from_queue(),
        // `d` deletes the focused playlist (Playlists view, col 0 only).
        // DEF-001: show a confirmation dialog before deletion — a single
        // accidental keypress must not destroy a playlist. When the guard
        // fails (not in Playlists view / col 0 / empty list), the arm falls
        // through to `_ => {}` (no-op), matching the old gating in
        // `delete_focused_playlist`.
        (KeyCode::Char('d'), _)
            if app.view == View::Playlists
                && app.focus_col == 0
                && app.playlists.get(app.cursors.playlist).is_some() =>
        {
            let name = app.playlists[app.cursors.playlist].name.clone();
            app.overlay = Some(Overlay::Confirm {
                message: format!("Delete playlist \"{name}\"?  y/n"),
                action: crate::tui::app::ConfirmAction::DeletePlaylist,
            });
        }

        // --- Overlays -------------------------------------------------------
        (KeyCode::Char('/'), _) => {
            // Default the search scope to the active view: YouTube search in
            // the Y view (explicit-submit — ytmusicapi is slow), local BM25
            // elsewhere (instant, live). `Tab` inside the overlay toggles.
            let scope = if app.view == crate::tui::app::View::Youtube {
                crate::tui::app::SearchScope::Youtube
            } else {
                crate::tui::app::SearchScope::Local
            };
            app.overlay = Some(Overlay::Search {
                input: String::new(),
                results: Vec::new(),
                cursor: 0,
                scope,
                submitted: None,
                searching: false,
            });
        }
        // `f` inline filter on the focused column (spec §5.4).
        (KeyCode::Char('f'), _) => app.toggle_filter(),
        (KeyCode::Char('?'), _) => {
            app.help_scroll = 0;
            app.overlay = Some(Overlay::Help);
        }
        (KeyCode::Char('a'), _) => {
            if let Some(track_id) = app.selected_track_id() {
                app.overlay = Some(Overlay::PlaylistPicker {
                    track_id,
                    cursor: 0,
                });
            }
        }
        (KeyCode::Char(':'), _) => {
            app.overlay = Some(Overlay::Command {
                input: String::new(),
                cursor: 0,
            });
        }
        // `L` toggles the lyrics overlay for the currently-playing track.
        // Shows loading → available (synced/plain) / not found / error, with
        // synced-line highlighting by player.position(). `L` again (or Esc) closes.
        (KeyCode::Char('L'), _) => app.toggle_lyrics(),
        // `D` opens the diagnostics overlay (recent provider errors, respawn
        // notices, sidecar failures). Esc closes. Also reachable via `:diag`.
        (KeyCode::Char('D'), _) => {
            app.overlay = Some(Overlay::Diagnostics);
        }
        // `H` opens the YouTube Home view (recommendation discovery). Also
        // reachable via `:home`.
        (KeyCode::Char('H'), _) => app.open_home(),

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
        Some(Overlay::Search {
            mut input,
            mut results,
            mut cursor,
            mut scope,
            mut submitted,
            mut searching,
        }) => {
            // Two search models, by scope:
            //   Local  — instant: typing re-runs the BM25 index live.
            //   Youtube — explicit-submit: typing only accumulates the query;
            //     Enter sends ONE async request (no per-keystroke network, which
            //     would stall the UI for ~3s/char). A second Enter on fresh
            //     results picks the track instead of re-searching.
            match key.code {
                // Tab toggles scope (local ↔ youtube) so the user can search the
                // local catalog from the Y view, or YouTube from a local view.
                // Results are scope-specific, so clear them on toggle.
                KeyCode::Tab => {
                    scope = match scope {
                        crate::tui::app::SearchScope::Local => {
                            crate::tui::app::SearchScope::Youtube
                        }
                        crate::tui::app::SearchScope::Youtube => {
                            crate::tui::app::SearchScope::Local
                        }
                    };
                    results.clear();
                    cursor = 0;
                    submitted = None;
                    searching = false;
                    // Local is instant: run the (now-local) query immediately.
                    if scope == crate::tui::app::SearchScope::Local && !input.is_empty() {
                        results = app.run_search_local(&input);
                        submitted = Some(input.clone());
                        if cursor >= results.len() {
                            cursor = results.len().saturating_sub(1);
                        }
                    }
                }
                // Enter: submit (Youtube dirty) or pick (results fresh) or pick
                // (Local, where results are always live).
                KeyCode::Enter => {
                    if searching {
                        // A request is already in flight; ignore.
                    } else if scope == crate::tui::app::SearchScope::Local {
                        if let Some(id) = results.get(cursor).cloned() {
                            let ids = std::mem::take(&mut results);
                            app.play_in_context_ids(ids, &id);
                            return;
                        }
                    } else {
                        // Youtube scope.
                        let fresh =
                            submitted.as_deref() == Some(input.as_str()) && !results.is_empty();
                        if fresh {
                            // Results match the current query → pick.
                            if let Some(id) = results.get(cursor).cloned() {
                                let ids = std::mem::take(&mut results);
                                app.play_in_context_ids(ids, &id);
                                return;
                            }
                        } else if !input.trim().is_empty() {
                            // Dirty / never submitted → fire-and-forget search.
                            app.submit_yt_search(input.clone());
                            submitted = Some(input.clone());
                            searching = true;
                            results.clear();
                            cursor = 0;
                        }
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
                    cursor = cursor
                        .checked_sub(1)
                        .unwrap_or(results.len().saturating_sub(1));
                }
                // Accept Char regardless of SHIFT so capital letters (and
                // shifted symbols) make it into the input — a Shift+F would
                // otherwise be dropped because its modifiers != NONE.
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    input.push(c);
                    if scope == crate::tui::app::SearchScope::Local {
                        // Local is instant: live-search on every keystroke.
                        results = app.run_search_local(&input);
                        submitted = Some(input.clone());
                        if cursor >= results.len() {
                            cursor = results.len().saturating_sub(1);
                        }
                    } else {
                        // Youtube: typing marks the query dirty (a later
                        // Enter re-submits). Don't touch results here — leave
                        // the stale list visible until the user submits.
                        searching = false;
                        // submitted no longer matches input → next Enter submits.
                    }
                }
                KeyCode::Backspace => {
                    input.pop();
                    if scope == crate::tui::app::SearchScope::Local {
                        results = app.run_search_local(&input);
                        submitted = Some(input.clone());
                        if cursor >= results.len() {
                            cursor = results.len().saturating_sub(1);
                        }
                    } else {
                        searching = false;
                    }
                }
                _ => {}
            }
            app.overlay = Some(Overlay::Search {
                input,
                results,
                cursor,
                scope,
                submitted,
                searching,
            });
        }
        Some(Overlay::Command {
            mut input,
            mut cursor,
        }) => {
            // Word-boundary helpers (byte-level — input is `:` commands, ASCII).
            fn word_start_left(s: &str, pos: usize) -> usize {
                let b = s.as_bytes();
                let mut i = pos.min(b.len());
                while i > 0 && (b[i - 1] as char).is_whitespace() {
                    i -= 1;
                }
                while i > 0 && !(b[i - 1] as char).is_whitespace() {
                    i -= 1;
                }
                i
            }
            fn word_start_right(s: &str, pos: usize) -> usize {
                let b = s.as_bytes();
                let mut i = pos.min(b.len());
                while i < b.len() && (b[i] as char).is_whitespace() {
                    i += 1;
                }
                while i < b.len() && !(b[i] as char).is_whitespace() {
                    i += 1;
                }
                i
            }
            match key.code {
                // Accept Char regardless of SHIFT — see the Search arm note.
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    cursor = cursor.min(input.len());
                    input.insert(cursor, c);
                    cursor += c.len_utf8();
                }
                KeyCode::Backspace if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    // Ctrl-Backspace: delete the word before the cursor.
                    cursor = cursor.min(input.len());
                    let start = word_start_left(&input, cursor);
                    input.drain(start..cursor);
                    cursor = start;
                }
                KeyCode::Backspace => {
                    cursor = cursor.min(input.len());
                    if cursor > 0 {
                        // Move to the previous char boundary (UTF-8 safe).
                        let mut prev = cursor - 1;
                        while prev > 0 && !input.is_char_boundary(prev) {
                            prev -= 1;
                        }
                        input.drain(prev..cursor);
                        cursor = prev;
                    }
                }
                KeyCode::Delete if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    // Ctrl-Delete: delete the word after the cursor.
                    cursor = cursor.min(input.len());
                    let end = word_start_right(&input, cursor);
                    input.drain(cursor..end);
                }
                KeyCode::Home => {
                    cursor = 0;
                }
                KeyCode::End => {
                    cursor = input.len();
                }
                KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    cursor = cursor.min(input.len());
                    cursor = word_start_left(&input, cursor);
                }
                KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    cursor = cursor.min(input.len());
                    cursor = word_start_right(&input, cursor);
                }
                KeyCode::Left => {
                    cursor = cursor.min(input.len());
                    if cursor > 0 {
                        // Move to the previous char boundary (UTF-8 safe).
                        let mut prev = cursor - 1;
                        while prev > 0 && !input.is_char_boundary(prev) {
                            prev -= 1;
                        }
                        cursor = prev;
                    }
                }
                KeyCode::Right => {
                    cursor = cursor.min(input.len());
                    if cursor < input.len() {
                        // Move to the next char boundary (UTF-8 safe).
                        let mut next = cursor + 1;
                        while next < input.len() && !input.is_char_boundary(next) {
                            next += 1;
                        }
                        cursor = next;
                    }
                }
                KeyCode::Tab => {
                    // Tab completion: complete known command prefix.
                    // NOTE: the stored `input` does NOT include the `:` prefix —
                    // `render_command` (overlay.rs) prepends `:` for display.
                    // Storing `:` here would double it (`::yt`) on screen (DEF-010).
                    let known = [
                        "queue", "yt", "lyrics", "diag", "help", "quit", "q", "home", "gen",
                        "radio", "publish",
                    ];
                    let prefix = input.trim_start_matches(':');
                    let matches: Vec<&str> = known
                        .iter()
                        .copied()
                        .filter(|c| c.starts_with(prefix))
                        .collect();
                    if matches.len() == 1 {
                        input = matches[0].to_string();
                        cursor = input.len();
                    } else if matches.len() > 1 {
                        // Complete the common prefix.
                        let common = matches
                            .iter()
                            .map(|s| s.as_bytes())
                            .fold(None::<Vec<u8>>, |acc, s| match acc {
                                None => Some(s.to_vec()),
                                Some(a) => {
                                    let len =
                                        a.iter().zip(s.iter()).take_while(|(x, y)| x == y).count();
                                    Some(a[..len].to_vec())
                                }
                            })
                            .unwrap_or_default();
                        input = String::from_utf8_lossy(&common).to_string();
                        cursor = input.len();
                    }
                }
                KeyCode::Up if !app.command_history.is_empty() => {
                    // Traverse command history backwards (most recent first).
                    if app.command_history_cursor.is_none() {
                        // Save the current draft before traversing.
                        app.command_draft = input.clone();
                        app.command_history_cursor = Some(0);
                    } else if let Some(i) = app.command_history_cursor {
                        if i + 1 < app.command_history.len() {
                            app.command_history_cursor = Some(i + 1);
                        }
                    }
                    if let Some(i) = app.command_history_cursor {
                        input = app.command_history[i].clone();
                        cursor = input.len();
                    }
                }
                KeyCode::Down => {
                    // Traverse command history forwards (toward the draft).
                    if let Some(i) = app.command_history_cursor {
                        if i == 0 {
                            // Past the end → restore the draft.
                            app.command_history_cursor = None;
                            input = app.command_draft.clone();
                            cursor = input.len();
                        } else {
                            app.command_history_cursor = Some(i - 1);
                            input = app.command_history[i - 1].clone();
                            cursor = input.len();
                        }
                    }
                }
                KeyCode::Enter => {
                    let cmd = input.trim().to_string();
                    app.overlay = None;
                    // Push to command history (bounded, adjacent-dedup).
                    if !cmd.is_empty()
                        && app.command_history.first().map(|s| s.as_str()) != Some(&cmd)
                    {
                        app.command_history.insert(0, cmd.clone());
                        if app.command_history.len() > 100 {
                            app.command_history.truncate(100);
                        }
                    }
                    app.command_history_cursor = None;
                    execute_command(app, &cmd);
                    return;
                }
                _ => {}
            }
            app.overlay = Some(Overlay::Command { input, cursor });
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
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
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
        // Help overlay: scroll the keymap (it's taller than the popup). Esc
        // (handled above) closes; j/k/↑/↓ scroll a line, PgUp/PgDn half a page,
        // g/G jump to top/bottom. Any other key is a no-op.
        Some(Overlay::Help) => {
            // Upper bound is the content length; over-scrolling just shows
            // blank space, so a generous constant is safe and avoids needing
            // the rendered height here.
            const HELP_LINES: u16 = 31;
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    app.help_scroll = app.help_scroll.saturating_add(1).min(HELP_LINES);
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    app.help_scroll = app.help_scroll.saturating_sub(1);
                }
                KeyCode::PageDown => {
                    app.help_scroll = app.help_scroll.saturating_add(10).min(HELP_LINES);
                }
                KeyCode::PageUp => {
                    app.help_scroll = app.help_scroll.saturating_sub(10);
                }
                KeyCode::Char('g') => app.help_scroll = 0,
                KeyCode::Char('G') => app.help_scroll = HELP_LINES,
                // Home/End jump to top/bottom (mirrors g/G) — DEF-018: End was
                // missing.
                KeyCode::Home => app.help_scroll = 0,
                KeyCode::End => app.help_scroll = HELP_LINES,
                _ => {}
            }
            app.overlay = Some(Overlay::Help);
        }
        // Help overlay: any non-Esc key is a no-op (overlay stays open until Esc).
        Some(Overlay::Discover { items, mut cursor }) => {
            // j/k mirror ↑↓ so the help-text keymap ("h j k l · ↑↓←→ move")
            // holds in the discover overlay too (DEF-026). Without this the
            // `_` arm swallowed j/k, leaving the cursor unchanged while the
            // renderer dropped the highlight — the overlay looked unselected.
            match key.code {
                KeyCode::Down | KeyCode::Char('j') if !items.is_empty() => {
                    cursor = (cursor + 1) % items.len();
                }
                KeyCode::Up | KeyCode::Char('k') if !items.is_empty() => {
                    cursor = cursor
                        .checked_sub(1)
                        .unwrap_or(items.len().saturating_sub(1));
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
        // `a` — playlist picker: j/k/↑↓ move, Enter confirms (add to existing
        // or create new), Esc cancels (handled at the top of this fn).
        Some(Overlay::PlaylistPicker {
            track_id,
            mut cursor,
        }) => {
            // Total rows = playlists + the "+ new playlist..." entry.
            let n = app.playlists.len() + 1;
            match key.code {
                KeyCode::Down | KeyCode::Char('j') if n > 0 => {
                    cursor = (cursor + 1) % n;
                }
                KeyCode::Up | KeyCode::Char('k') if n > 0 => {
                    cursor = cursor.checked_sub(1).unwrap_or(n.saturating_sub(1));
                }
                KeyCode::Enter => {
                    if cursor < app.playlists.len() {
                        app.add_track_to_playlist(&track_id, cursor);
                        app.save_playlists_db();
                        app.yt_status =
                            Some(format!("added to \"{}\"", app.playlists[cursor].name));
                        app.overlay = None;
                        return;
                    } else {
                        // "+ new playlist..." — DEF-014: prompt for a name
                        // before creating instead of auto-naming immediately.
                        app.overlay = Some(Overlay::TextInput {
                            prompt: "New playlist name:".to_string(),
                            buffer: String::new(),
                            cursor: 0,
                            action: crate::tui::app::TextInputAction::NewPlaylist {
                                track_id: track_id.clone(),
                            },
                        });
                        return;
                    }
                }
                _ => {}
            }
            app.overlay = Some(Overlay::PlaylistPicker { track_id, cursor });
        }
        // Lyrics overlay (`L`): j/k/↑/↓/PgUp/PgDn/g/G scroll; Esc closes
        // (handled at the top of this fn). `L` toggles closed (same as Esc).
        Some(Overlay::Lyrics {
            content,
            state,
            mut scroll,
            track_id,
            gen,
        }) => {
            if matches!(key.code, KeyCode::Char('R'))
                && matches!(
                    state,
                    crate::tui::app::LyricsState::NotFound
                        | crate::tui::app::LyricsState::Offline
                        | crate::tui::app::LyricsState::Error(_)
                )
            {
                app.overlay = Some(Overlay::Lyrics {
                    content,
                    state,
                    scroll,
                    track_id: track_id.clone(),
                    gen,
                });
                app.request_lyrics(&track_id);
                return;
            }
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    scroll = scroll.saturating_add(1);
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    scroll = scroll.saturating_sub(1);
                }
                KeyCode::PageDown => {
                    scroll = scroll.saturating_add(10);
                }
                KeyCode::PageUp => {
                    scroll = scroll.saturating_sub(10);
                }
                KeyCode::Char('g') => scroll = 0,
                KeyCode::Char('G') => scroll = u16::MAX,
                KeyCode::Char('L') => {
                    app.overlay = None;
                    return;
                }
                _ => {}
            }
            app.overlay = Some(Overlay::Lyrics {
                content,
                state,
                scroll,
                track_id,
                gen,
            });
        }
        Some(Overlay::Diagnostics) => {
            // Esc closes the diagnostics overlay; all other keys are
            // swallowed (no interaction while the diag list is open).
            if key.code == KeyCode::Esc {
                app.overlay = None;
                return;
            }
            app.overlay = Some(Overlay::Diagnostics);
        }
        // Confirmation dialog (DEF-001, DEF-015): y/Enter confirms the
        // action, n/Esc cancels. Any other key is a no-op (overlay stays).
        Some(Overlay::Confirm { message, action }) => {
            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    match action {
                        crate::tui::app::ConfirmAction::DeletePlaylist => {
                            app.delete_focused_playlist();
                        }
                        crate::tui::app::ConfirmAction::YtLogout => {
                            app.yt_logout();
                        }
                        crate::tui::app::ConfirmAction::ClearQueue => {
                            app.transport.clear_queue();
                            app.yt_status = Some("queue cleared".into());
                        }
                    }
                    app.overlay = None;
                    return;
                }
                KeyCode::Char('n') => {
                    app.overlay = None;
                    return;
                }
                _ => {}
            }
            app.overlay = Some(Overlay::Confirm { message, action });
        }
        // Text input overlay (DEF-014: playlist name). Typing accumulates
        // in `buffer`; Enter creates the playlist; Esc cancels (handled at
        // the top of this fn). Backspace/Left/Right/Home/End edit.
        Some(Overlay::TextInput {
            prompt,
            mut buffer,
            mut cursor,
            action,
        }) => {
            match key.code {
                KeyCode::Enter => {
                    match action {
                        crate::tui::app::TextInputAction::NewPlaylist { track_id } => {
                            let trimmed = buffer.trim();
                            if trimmed.is_empty() {
                                // Empty input → fall back to auto-name.
                                let idx = app.create_playlist_with_track(&track_id);
                                app.save_playlists_db();
                                app.yt_status =
                                    Some(format!("created \"{}\"", app.playlists[idx].name));
                            } else {
                                use crate::tui::app::Playlist;
                                app.playlists.push(Playlist {
                                    name: trimmed.to_string(),
                                    track_ids: vec![track_id.clone()],
                                });
                                app.save_playlists_db();
                                app.yt_status = Some(format!("created \"{trimmed}\""));
                            }
                        }
                    }
                    app.overlay = None;
                    return;
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    cursor = cursor.min(buffer.len());
                    buffer.insert(cursor, c);
                    cursor += c.len_utf8();
                }
                KeyCode::Backspace => {
                    cursor = cursor.min(buffer.len());
                    if cursor > 0 {
                        let mut prev = cursor - 1;
                        while prev > 0 && !buffer.is_char_boundary(prev) {
                            prev -= 1;
                        }
                        buffer.drain(prev..cursor);
                        cursor = prev;
                    }
                }
                KeyCode::Left => {
                    cursor = cursor.min(buffer.len());
                    if cursor > 0 {
                        let mut prev = cursor - 1;
                        while prev > 0 && !buffer.is_char_boundary(prev) {
                            prev -= 1;
                        }
                        cursor = prev;
                    }
                }
                KeyCode::Right => {
                    cursor = cursor.min(buffer.len());
                    if cursor < buffer.len() {
                        let mut next = cursor + 1;
                        while next < buffer.len() && !buffer.is_char_boundary(next) {
                            next += 1;
                        }
                        cursor = next;
                    }
                }
                KeyCode::Home => {
                    cursor = 0;
                }
                KeyCode::End => {
                    cursor = buffer.len();
                }
                _ => {}
            }
            app.overlay = Some(Overlay::TextInput {
                prompt,
                buffer,
                cursor,
                action,
            });
        }
        // --- Recommendation overlays -----------------------------------------
        // Home overlay (`H` / `:home`): j/k navigate items, Tab switches
        // sections, Enter selects the highlighted item, Esc closes (generic).
        Some(Overlay::Home { mut state }) => {
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    state.cursor_down(usize::MAX);
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    state.cursor_up();
                }
                KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    state.section_prev();
                }
                KeyCode::Tab => {
                    state.section_next(crate::tui::view::home::HomeSection::all().len());
                }
                KeyCode::Enter => {
                    // Select the highlighted item. The Home overlay's items are
                    // populated by `open_home`; selection dispatches back to
                    // the app (play a mix, open a playlist, start radio). Close
                    // the overlay and play the selected context track.
                    app.overlay = Some(Overlay::Home { state });
                    app.play_selected();
                    return;
                }
                _ => {}
            }
            app.overlay = Some(Overlay::Home { state });
        }
        // Radio overlay (`:radio`): +/- feedback, s skip, n next, Esc stops.
        Some(Overlay::Radio { session }) => {
            // The current track is what's playing now (the radio's last pick).
            let track_id = app.now_playing.as_ref().map(|ts| ts.id().to_string());
            match key.code {
                KeyCode::Char('+') | KeyCode::Char('=') => {
                    if let Some(id) = &track_id {
                        app.apply_reco_feedback(FeedbackAction::Like, id);
                    }
                    // Positive feedback doesn't advance; the user stays on
                    // the track they like.
                }
                KeyCode::Char('-') => {
                    if let Some(id) = &track_id {
                        app.apply_reco_feedback(FeedbackAction::HideTrack, id);
                    }
                    app.next();
                }
                KeyCode::Char('s') => {
                    if let Some(id) = &track_id {
                        app.apply_reco_feedback(FeedbackAction::RemoveFromMix, id);
                    }
                    app.next();
                }
                KeyCode::Char('n') => {
                    app.next();
                }
                _ => {}
            }
            app.overlay = Some(Overlay::Radio { session });
        }
        // Generator overlay (`G` / `:gen`): NL input -> plan -> preview. In
        // the Input phase, typing accumulates in `state.input`; Enter
        // generates. In the Preview phase, p/x pin/remove, g regenerates, e
        // edits, j/k navigate, Enter saves.
        Some(Overlay::Generator { mut state }) => {
            use crate::tui::view::generator::GeneratorPhase;
            match key.code {
                // Input phase: typing accumulates into the NL query.
                KeyCode::Char(c)
                    if state.phase == GeneratorPhase::Input
                        && !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    state.input.push(c);
                }
                KeyCode::Backspace if state.phase == GeneratorPhase::Input => {
                    state.input.pop();
                }
                // Enter: Input/ReviewPlan -> generate; Preview -> save.
                KeyCode::Enter
                    if state.phase == GeneratorPhase::Input
                        || state.phase == GeneratorPhase::ReviewPlan =>
                {
                    app.overlay = Some(Overlay::Generator { state });
                    app.generate_playlist();
                    return;
                }
                KeyCode::Enter if state.phase == GeneratorPhase::Preview => {
                    // Save the generated playlist locally.
                    if let Some(p) = &state.playlist {
                        let track_ids: Vec<String> =
                            p.tracks.iter().map(|t| t.track_id.clone()).collect();
                        let name = if !state.input.trim().is_empty() {
                            state.input.trim().to_string()
                        } else {
                            "Generated Playlist".to_string()
                        };
                        app.playlists
                            .push(crate::tui::app::Playlist { name, track_ids });
                        app.save_playlists_db();
                        app.yt_status = Some("playlist saved".into());
                    }
                    app.overlay = None;
                    return;
                }
                // Preview phase: p pin, x remove, g regenerate, e edit.
                KeyCode::Char('p') if state.phase == GeneratorPhase::Preview => {
                    let tid = state
                        .playlist
                        .as_ref()
                        .and_then(|p| p.tracks.get(state.cursor))
                        .map(|t| t.track_id.clone());
                    if let Some(id) = tid {
                        state.pin_track(id);
                    }
                }
                KeyCode::Char('x') if state.phase == GeneratorPhase::Preview => {
                    let tid = state
                        .playlist
                        .as_ref()
                        .and_then(|p| p.tracks.get(state.cursor))
                        .map(|t| t.track_id.clone());
                    if let Some(id) = tid {
                        state.remove_track(&id);
                    }
                }
                KeyCode::Char('g') if state.phase == GeneratorPhase::Preview => {
                    app.overlay = Some(Overlay::Generator { state });
                    app.generate_playlist();
                    return;
                }
                KeyCode::Char('e') if state.phase == GeneratorPhase::ReviewPlan => {
                    state.phase = GeneratorPhase::Input;
                }
                // Preview phase: j/k navigate the track list.
                KeyCode::Down | KeyCode::Char('j') if state.phase == GeneratorPhase::Preview => {
                    if let Some(p) = &state.playlist {
                        if state.cursor + 1 < p.tracks.len() {
                            state.cursor += 1;
                        }
                    }
                }
                KeyCode::Up | KeyCode::Char('k') if state.phase == GeneratorPhase::Preview => {
                    state.cursor = state.cursor.saturating_sub(1);
                }
                _ => {}
            }
            app.overlay = Some(Overlay::Generator { state });
        }
        // Explanation overlay: Esc closes (generic). Any other key is a no-op
        // — the overlay stays until the user reads and closes it.
        Some(Overlay::Explanation { explanation }) => {
            app.overlay = Some(Overlay::Explanation { explanation });
        }
        // Publication overlay (`:publish`): Tab cycles privacy, Enter
        // proceeds, y confirms, n cancels, Esc closes (generic).
        Some(Overlay::Publication { mut state }) => {
            match key.code {
                KeyCode::Tab => {
                    state.privacy = match state.privacy.as_str() {
                        "PRIVATE" => "UNLISTED".into(),
                        "UNLISTED" => "PUBLIC".into(),
                        _ => "PRIVATE".into(),
                    };
                }
                KeyCode::Enter => {
                    if state.is_ready() {
                        state.confirmed = true;
                        app.overlay = None;
                        app.yt_status = Some(format!("publishing \"{}\" to YouTube", state.name));
                    } else {
                        state.step = state.step.saturating_add(1);
                    }
                }
                KeyCode::Char('y') if state.is_ready() => {
                    state.confirmed = true;
                    app.overlay = None;
                    app.yt_status = Some(format!("publishing \"{}\" to YouTube", state.name));
                }
                KeyCode::Char('n') => {
                    app.overlay = None;
                    return;
                }
                _ => {}
            }
            app.overlay = Some(Overlay::Publication { state });
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
        KeyCode::Char(c)
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT) =>
        {
            // `f` on an empty filter closes it (toggle semantics); otherwise
            // the char goes into the query. Backspace-to-empty + `f` also
            // closes, so you don't get a stranded empty filter.
            if c == 'f'
                && app
                    .filter
                    .as_ref()
                    .map(|f| f.text.is_empty())
                    .unwrap_or(false)
            {
                app.filter = None;
            } else if let Some(f) = &mut app.filter {
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

/// Execute a `:` command. Supports `:yt auth`, `:yt auth browser <name>`,
/// `:yt logout`, `:yt setup`.
fn execute_command(app: &mut App, cmd: &str) {
    match cmd {
        "yt auth" => {
            app.overlay = Some(Overlay::YtAuth {
                input: String::new(),
            });
        }
        "yt logout" => {
            // DEF-015: show a confirmation dialog before clearing credentials.
            app.overlay = Some(Overlay::Confirm {
                message: "Clear YouTube credentials and log out?  y/n".to_string(),
                action: crate::tui::app::ConfirmAction::YtLogout,
            });
        }
        "yt setup" => {
            app.yt_setup();
        }
        "queue clear" => {
            // MOD-4: confirm before clearing a non-empty queue (mirrors the
            // `d` delete-playlist and `:yt logout` confirmation pattern from
            // DEF-001 / DEF-015). When the queue is already empty, `:queue
            // clear` is a no-op without confirmation — there's nothing to
            // destroy.
            if app.transport.manual_queue.is_empty() {
                app.yt_status = Some("queue is empty".into());
            } else {
                app.overlay = Some(Overlay::Confirm {
                    message: "Clear the play-next queue?  y/n".to_string(),
                    action: crate::tui::app::ConfirmAction::ClearQueue,
                });
            }
        }
        // `:diag` — open the diagnostics overlay (recent provider errors,
        // respawn notices, sidecar failures). Esc closes (generic handler).
        "diag" => {
            app.overlay = Some(Overlay::Diagnostics);
        }
        // `:q` / `:quit` — quit the app (same as the `q` keybinding).
        "q" | "quit" => {
            app.quit();
        }
        // `:home` — open the YouTube Home view (same as the `H` keybinding).
        "home" => {
            app.open_home();
        }
        // `:gen` — open the playlist generator (same as `G` in the Y view).
        "gen" => {
            app.open_generator();
        }
        // `:radio` — start a radio session from the currently selected track.
        "radio" => {
            if let Some(track_id) = app.selected_track_id() {
                app.start_radio_from_track(&track_id);
            } else {
                app.yt_status = Some("no track selected".into());
            }
        }
        // `:publish` — start the publication flow for the focused playlist.
        "publish" => {
            if let Some(name) = app
                .playlists
                .get(app.cursors.playlist)
                .map(|p| p.name.clone())
            {
                app.open_publication(&name);
            } else {
                app.yt_status = Some("no playlist selected".into());
            }
        }
        other if other.starts_with("yt auth browser") => {
            let browser = other
                .trim_start_matches("yt auth browser")
                .trim()
                .to_string();
            if browser.is_empty() {
                app.yt_error = Some(
                    "usage: :yt auth browser <chrome|firefox|safari|edge|brave|opera|chromium>"
                        .into(),
                );
            } else {
                app.apply_yt_browser(browser);
            }
        }
        // `:radio artist <name>` — start a radio session from an artist.
        other if other.starts_with("radio artist ") => {
            let artist = other.trim_start_matches("radio artist ").trim().to_string();
            if artist.is_empty() {
                app.yt_status = Some("usage: :radio artist <name>".into());
            } else {
                app.start_radio_from_artist(&artist);
            }
        }
        // `:publish <playlist>` — start the publication flow for a named
        // playlist.
        other if other.starts_with("publish ") => {
            let name = other.trim_start_matches("publish ").trim().to_string();
            if name.is_empty() {
                app.yt_status = Some("usage: :publish <playlist>".into());
            } else {
                app.open_publication(&name);
            }
        }
        _ => {
            // Unknown command — provide feedback so the user isn't left
            // wondering if their command ran. Known commands are matched
            // above; anything else (non-empty) is unknown.
            if !cmd.is_empty() {
                app.yt_error = Some(format!("unknown command: :{cmd}"));
            }
        }
    }
}
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
            let artist = app
                .artists
                .get(app.cursors.artist)
                .cloned()
                .unwrap_or_default();
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
    // Clamp cursors for the target view so a stale cursor from the previous
    // view (e.g. `cursors.playlist` pointing into `yt_lists` while switching
    // to Artists/Playlists, or vice-versa) doesn't leave a column empty on
    // the first render. `layout::draw` also clamps each frame, but doing it
    // here too means the very first frame after the switch is already
    // correct — no 1-frame flicker where the Tracks pane reads "no album
    // selected" because the album cursor was past the end.
    app.clamp_cursors();
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
    let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
    handle_mouse_in_area(app, m, ratatui::layout::Rect::new(0, 0, width, height));
}

/// Deterministic mouse dispatcher for a known frame area.
pub fn handle_mouse_in_area(app: &mut App, m: MouseEvent, area: ratatui::layout::Rect) {
    match m.kind {
        MouseEventKind::ScrollUp => move_up(app),
        MouseEventKind::ScrollDown => move_down(app),
        MouseEventKind::Down(_) => {
            // A single click — route to the player bar (transport/seek) or the
            // browse columns. Drag is deliberately NOT routed: a held-drag used
            // to scrub volume on every mouse-move, which jumped the level
            // erratically. Volume is keyboard-only (+/-/m) now.
            let bar = crate::tui::view::layout::player_bar_area(area);
            if let Some(bar) = bar.filter(|bar| rect_contains(*bar, m.column, m.row)) {
                handle_player_bar_click(app, m.column, m.row, bar);
            } else if bar.is_none_or(|bar| m.row < bar.y) {
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
fn rect_contains(rect: ratatui::layout::Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.right() && row >= rect.y && row < rect.bottom()
}

fn handle_player_bar_click(app: &mut App, col: u16, row: u16, area: ratatui::layout::Rect) {
    let geo = crate::tui::view::player_bar::geometry(area);
    if rect_contains(geo.progress, col, row) {
        let pct = (col.saturating_sub(geo.progress.x) as f64 / geo.progress.width.max(1) as f64)
            .clamp(0.0, 1.0);
        if let Some(dur) = app.player.duration() {
            if dur > 0.0 {
                let _ = app
                    .player
                    .seek(pct * dur - app.player.position().unwrap_or(0.0));
            }
        }
        return;
    }
    if rect_contains(geo.previous, col, row) {
        app.prev();
    } else if rect_contains(geo.play_pause, col, row) {
        let _ = app.player.play_pause();
    } else if rect_contains(geo.next, col, row) {
        app.next();
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
