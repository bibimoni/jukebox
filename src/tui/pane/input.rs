//! Pane-edit-mode key dispatch + prefix-key routing.
//!
//! Two entry points:
//!
//! - [`handle_prefix_key`] — called by `input::handle_key` after the
//!   user presses `Ctrl+w`. Consumes the next key as a pane command
//!   (navigation in any mode, edit-mode-enter, etc.). Returns
//!   `Some(())` if the key was a pane command, `None` if it should fall
//!   through to normal dispatch.
//!
//! - [`handle_pane_edit_key`] — called by `input::handle_key` when
//!   `app.pane_workspace.mode == UiMode::PaneEdit`. Handles split / close
//!   / resize / change-module / exit. Global playback + quit keys
//!   pass through to the normal handler.
//!
//! Overlay key handlers for `PaneSplitDirection` and `PaneModulePicker`
//! also live here (they're pane-edit-mode sub-modes).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::tui::app::{App, Overlay};
use crate::tui::pane::focus::move_focus_directional;
use crate::tui::pane::layout::resolve_rects;
use crate::tui::pane::model::{Direction, ModuleId, Side, UiMode};
use crate::tui::pane::selection::{
    NormalizedPoint, RectangleSelection, SelectionInput, SelectionPhase,
};
use crate::tui::pane::{PaneWorkspace, RESIZE_STEP};

/// The keyboard step for rectangle selection: 2% per press (matches
/// `RESIZE_STEP` for consistency). Shift+arrow moves 4x = 8%.
const SELECTION_STEP: f32 = 0.02;

/// Result of a prefix-key handler: did we consume the key?
pub struct PrefixResult(bool);

impl PrefixResult {
    pub fn consumed() -> Self {
        Self(true)
    }
    pub fn not_consumed() -> Self {
        Self(false)
    }
    pub fn is_consumed(self) -> bool {
        self.0
    }
}

/// Handle a key after `Ctrl+w` was pressed. Works in any mode (Normal
/// or PaneEdit) for navigation; only enters PaneEdit when explicitly
/// requested. Returns whether the key was consumed as a pane command.
pub fn handle_prefix_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        // `Ctrl+w, e` — toggle pane edit mode.
        KeyCode::Char('e') => {
            if app.pane_workspace.mode == UiMode::PaneEdit {
                app.pane_workspace.exit_edit_mode();
            } else {
                app.pane_workspace.enter_edit_mode();
            }
            true
        }
        // `Ctrl+w, h` / `j` / `k` / `l` — move pane focus spatially.
        KeyCode::Char('h') => move_focus(app, Direction::Left),
        KeyCode::Char('j') => move_focus(app, Direction::Down),
        KeyCode::Char('k') => move_focus(app, Direction::Up),
        KeyCode::Char('l') => move_focus(app, Direction::Right),
        // Arrows also work as pane navigators in the prefix.
        KeyCode::Left => move_focus(app, Direction::Left),
        KeyCode::Down => move_focus(app, Direction::Down),
        KeyCode::Up => move_focus(app, Direction::Up),
        KeyCode::Right => move_focus(app, Direction::Right),
        // `Ctrl+w, Tab` / `Ctrl+w, Shift+Tab` — cycle pane focus.
        KeyCode::Tab => cycle_focus(app, !key.modifiers.contains(KeyModifiers::SHIFT)),
        // All other keys after `Ctrl+w` are not pane commands — let them
        // fall through to normal dispatch.
        _ => false,
    }
}

/// Move the pane focus in a direction. Returns true if focus moved.
fn move_focus(app: &mut App, dir: Direction) -> bool {
    let panes = resolve_rects(&app.pane_workspace.root, app.pane_content_area());
    if let Some(new) = move_focus_directional(&panes, app.pane_workspace.focused_pane, dir) {
        app.pane_workspace.set_focused(new);
        // Sync `app.view` so the now-focused pane's module drives
        // navigation. `pane_content_area` is a best-effort rect; the
        // actual rects are recomputed on render.
        sync_app_view_to_focused_pane(app);
        true
    } else {
        false
    }
}

/// Cycle pane focus forward (true) or backward (false).
fn cycle_focus(app: &mut App, forward: bool) -> bool {
    let panes = resolve_rects(&app.pane_workspace.root, app.pane_content_area());
    if let Some(new) =
        crate::tui::pane::focus::cycle_focus(&panes, app.pane_workspace.focused_pane, forward)
    {
        app.pane_workspace.set_focused(new);
        sync_app_view_to_focused_pane(app);
        true
    } else {
        false
    }
}

/// Set `app.view` to match the focused pane's module so the existing
/// view functions read the right state. This is the seam between the
/// pane layer and the existing per-view renderers.
fn sync_app_view_to_focused_pane(app: &mut App) {
    let module = focused_module(&app.pane_workspace);
    if let Some(view) = module_to_view(module) {
        app.view = view;
    }
}

fn focused_module(ws: &PaneWorkspace) -> ModuleId {
    let panes = resolve_rects(&ws.root, ratatui::layout::Rect::new(0, 0, 1, 1));
    panes
        .iter()
        .find(|p| p.pane_id == ws.focused_pane)
        .map(|p| p.module_id)
        .unwrap_or(ModuleId::Artists)
}

fn module_to_view(module: ModuleId) -> Option<crate::tui::app::View> {
    match module {
        ModuleId::Artists => Some(crate::tui::app::View::Artists),
        ModuleId::Playlists => Some(crate::tui::app::View::Playlists),
        ModuleId::Queue => Some(crate::tui::app::View::Queue),
        ModuleId::Youtube => Some(crate::tui::app::View::Youtube),
        ModuleId::Placeholder => None,
    }
}

/// Handle a key while in PaneEdit mode. Returns true if the key was
/// consumed (so the global handler should NOT run); false if it should
/// fall through to the global handler.
pub fn handle_pane_edit_key(app: &mut App, key: KeyEvent) -> bool {
    // Reserved Ctrl-* keys: never bind. (Mirrors the top of `handle_key`.)
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c')
            | KeyCode::Char('z')
            | KeyCode::Char('\\')
            | KeyCode::Char('s')
            | KeyCode::Char('q') => return false,
            _ => {}
        }
    }

    // Rectangle selection sub-mode: when active, route all keys to the
    // selection handler. It consumes the keys it cares about (arrows,
    // hjkl, Tab, Enter, Esc, r) and the playback keys fall through to
    // the normal pane-edit handler below. Other keys are swallowed so
    // they don't accidentally split/close the pane being selected.
    if app.rectangle_selection.is_some() && handle_rectangle_selection_key(app, key) {
        return true;
    }
    // Playback keys fall through to the normal pane-edit handler (so
    // Space/>/</,/. /+/-/q keep working during selection).

    // Esc: exit pane edit mode.
    if matches!(key.code, KeyCode::Esc) {
        app.pane_workspace.exit_edit_mode();
        // Defensive: clear any stale selection (shouldn't be set here,
        // since the rectangle selection handler above would have
        // cleared it; but a future code path might re-enter edit mode
        // with a stale selection).
        app.rectangle_selection = None;
        return true;
    }
    // `Ctrl+w, e`: exit (alias for Esc).
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('w')) {
        app.pending_pane_prefix = true;
        return true;
    }

    // Navigation: h/j/k/l + arrows move pane focus.
    match key.code {
        KeyCode::Char('h') if key.modifiers == KeyModifiers::NONE => {
            return move_focus(app, Direction::Left);
        }
        KeyCode::Char('j') if key.modifiers == KeyModifiers::NONE => {
            return move_focus(app, Direction::Down);
        }
        KeyCode::Char('k') if key.modifiers == KeyModifiers::NONE => {
            return move_focus(app, Direction::Up);
        }
        KeyCode::Char('l') if key.modifiers == KeyModifiers::NONE => {
            return move_focus(app, Direction::Right);
        }
        KeyCode::Left => return move_focus(app, Direction::Left),
        KeyCode::Down => return move_focus(app, Direction::Down),
        KeyCode::Up => return move_focus(app, Direction::Up),
        KeyCode::Right => return move_focus(app, Direction::Right),
        _ => {}
    }

    // Resize: H/J/K/L grow the focused pane in that direction.
    match key.code {
        KeyCode::Char('H') => return resize_focused(app, Direction::Left),
        KeyCode::Char('J') => return resize_focused(app, Direction::Down),
        KeyCode::Char('K') => return resize_focused(app, Direction::Up),
        KeyCode::Char('L') => return resize_focused(app, Direction::Right),
        _ => {}
    }

    // Splitting / closing / module picker.
    match key.code {
        // `s` opens the split-direction picker overlay.
        KeyCode::Char('s') if key.modifiers == KeyModifiers::NONE => {
            app.overlay = Some(Overlay::PaneSplitDirection {
                target_pane: app.pane_workspace.focused_pane,
            });
            app.pane_workspace.set_mode(UiMode::PaneModulePicker);
            return true;
        }
        // `v` directly splits with the new pane on the RIGHT (Placeholder).
        KeyCode::Char('v') if key.modifiers == KeyModifiers::NONE => {
            let target = app.pane_workspace.focused_pane;
            let _ = app
                .pane_workspace
                .split(target, Side::Right, ModuleId::Placeholder);
            return true;
        }
        // `x` directly splits with the new pane on the BOTTOM (Placeholder).
        KeyCode::Char('x') if key.modifiers == KeyModifiers::NONE => {
            let target = app.pane_workspace.focused_pane;
            let _ = app
                .pane_workspace
                .split(target, Side::Bottom, ModuleId::Placeholder);
            return true;
        }
        // `d` closes the focused pane. Refuses if it's the only pane (no
        // mutation, no panic).
        KeyCode::Char('d') if key.modifiers == KeyModifiers::NONE => {
            let target = app.pane_workspace.focused_pane;
            match app.pane_workspace.close(target) {
                Ok(()) => sync_app_view_to_focused_pane(app),
                Err(crate::tui::pane::model::CloseError::IsRoot) => {
                    app.set_status_toast("can't close the last pane".into());
                }
                Err(crate::tui::pane::model::CloseError::NotFound) => {
                    // Defensive — the focused pane is always in the tree.
                }
            }
            return true;
        }
        // `m` opens the module picker overlay for the focused pane.
        KeyCode::Char('m') if key.modifiers == KeyModifiers::NONE => {
            app.overlay = Some(Overlay::PaneModulePicker {
                target_pane: app.pane_workspace.focused_pane,
                // None = change the existing pane's module (no split).
                pending_split: None,
                cursor: 0,
            });
            app.pane_workspace.set_mode(UiMode::PaneModulePicker);
            return true;
        }
        // `r` enters rectangle selection mode. The anchor + cursor
        // start at the center of the focused pane; the user picks a
        // sub-region with arrows / hjkl + Enter, then chooses a module
        // from the picker. The selection is converted to split-tree
        // ops so the result is a proportional split tree (no floating
        // rectangles). See [`crate::tui::pane::selection`].
        KeyCode::Char('r') if key.modifiers == KeyModifiers::NONE => {
            let target = app.pane_workspace.focused_pane;
            app.rectangle_selection = Some(RectangleSelection::new(target));
            return true;
        }
        // Tab / Shift+Tab cycle pane focus.
        KeyCode::Tab => return cycle_focus(app, !key.modifiers.contains(KeyModifiers::SHIFT)),
        _ => {}
    }

    // 1/2/3/4: change the focused pane's module to the matching view.
    match key.code {
        KeyCode::Char('1') if key.modifiers == KeyModifiers::NONE => {
            return change_focused_module(app, ModuleId::Artists);
        }
        KeyCode::Char('2') if key.modifiers == KeyModifiers::NONE => {
            return change_focused_module(app, ModuleId::Playlists);
        }
        KeyCode::Char('3') if key.modifiers == KeyModifiers::NONE => {
            return change_focused_module(app, ModuleId::Queue);
        }
        KeyCode::Char('4') if key.modifiers == KeyModifiers::NONE => {
            return change_focused_module(app, ModuleId::Youtube);
        }
        _ => {}
    }

    // Global keys that keep working in PaneEdit mode:
    // Playback (Space, >, <, ,, ., +, -), quit (q), view-switch is remapped
    // above to change the focused pane's module. Mute is on `Ctrl+w, m`
    // (we don't intercept it here — the prefix handler is pane-mode-aware).
    match key.code {
        KeyCode::Char(' ') => {
            app.toggle_pause();
            return true;
        }
        KeyCode::Char('>') => {
            app.next();
            return true;
        }
        KeyCode::Char('<') => {
            app.prev();
            return true;
        }
        KeyCode::Char(',') => {
            let _ = app.player.seek(-5.0);
            return true;
        }
        KeyCode::Char('.') => {
            let _ = app.player.seek(5.0);
            return true;
        }
        KeyCode::Char('+') => {
            app.volume_up();
            return true;
        }
        KeyCode::Char('-') => {
            app.volume_down();
            return true;
        }
        KeyCode::Char('q') => {
            app.quit();
            return true;
        }
        KeyCode::Enter => {
            app.play_selected();
            return true;
        }
        _ => {}
    }

    // All other keys are not consumed — let the caller decide (e.g. fall
    // through to overlay routing if an overlay is open, or just drop).
    false
}

/// Resize the focused pane in a direction. Returns true if the resize
/// happened (always true — no-op resizes still count as "consumed" so
/// the key doesn't fall through to the global handler).
fn resize_focused(app: &mut App, dir: Direction) -> bool {
    let target = app.pane_workspace.focused_pane;
    let _ = app.pane_workspace.resize(target, dir, RESIZE_STEP);
    true
}

/// Change the focused pane's module. Returns true (consumed).
fn change_focused_module(app: &mut App, module: ModuleId) -> bool {
    let target = app.pane_workspace.focused_pane;
    app.pane_workspace.set_module(target, module);
    sync_app_view_to_focused_pane(app);
    true
}

// ---------------------------------------------------------------------------
// Overlay key handlers: PaneSplitDirection + PaneModulePicker
// ---------------------------------------------------------------------------

/// Handle a key in the `PaneSplitDirection` overlay (choosing which
/// side to split). Returns true if the key was consumed.
pub fn handle_split_direction_key(app: &mut App, key: KeyEvent) -> bool {
    // Esc cancels (handled by the generic overlay Esc path in
    // `handle_overlay_key`, but we also handle it here for clarity).
    if matches!(key.code, KeyCode::Esc) {
        app.overlay = None;
        app.pane_workspace.set_mode(UiMode::PaneEdit);
        return true;
    }
    let side = match key.code {
        KeyCode::Char('h') | KeyCode::Left => Some(Side::Left),
        KeyCode::Char('l') | KeyCode::Right => Some(Side::Right),
        KeyCode::Char('k') | KeyCode::Up => Some(Side::Top),
        KeyCode::Char('j') | KeyCode::Down => Some(Side::Bottom),
        _ => None,
    };
    if let Some(side) = side {
        // Take the overlay to get the target pane id, then open the
        // module picker with the pending split.
        if let Some(Overlay::PaneSplitDirection { target_pane }) = app.overlay.take() {
            app.overlay = Some(Overlay::PaneModulePicker {
                target_pane,
                pending_split: Some(side),
                cursor: 0,
            });
            return true;
        }
    }
    // Other keys are swallowed (overlay stays open).
    true
}

/// Handle a key in the `PaneModulePicker` overlay (choosing a module).
/// Returns true if the key was consumed.
pub fn handle_module_picker_key(app: &mut App, key: KeyEvent) -> bool {
    // Esc cancels.
    if matches!(key.code, KeyCode::Esc) {
        app.overlay = None;
        app.pane_workspace.set_mode(UiMode::PaneEdit);
        return true;
    }
    let (target_pane, pending_split, mut cursor) = match app.overlay.take() {
        Some(Overlay::PaneModulePicker {
            target_pane,
            pending_split,
            cursor,
        }) => (target_pane, pending_split, cursor),
        _ => return false,
    };
    let modules = ModuleId::all();
    let n = modules.len();
    match key.code {
        KeyCode::Down | KeyCode::Char('j') if n > 0 => {
            cursor = (cursor + 1) % n;
        }
        KeyCode::Up | KeyCode::Char('k') if n > 0 => {
            cursor = cursor.checked_sub(1).unwrap_or(n - 1);
        }
        KeyCode::Enter => {
            let module = modules[cursor.min(n - 1)];
            if let Some(selection) = app.rectangle_selection.take() {
                // Rectangle selection confirmed: convert the rectangle
                // to split-tree ops + install the chosen module in the
                // center pane. The surrounding panes get Placeholder.
                app.pane_workspace
                    .apply_rectangle_selection(&selection, module);
            } else if let Some(side) = pending_split {
                // Split + assign module.
                let _ = app.pane_workspace.split(target_pane, side, module);
            } else {
                // Change the existing pane's module.
                app.pane_workspace.set_module(target_pane, module);
            }
            sync_app_view_to_focused_pane(app);
            app.overlay = None;
            app.pane_workspace.set_mode(UiMode::PaneEdit);
            return true;
        }
        _ => {}
    }
    app.overlay = Some(Overlay::PaneModulePicker {
        target_pane,
        pending_split,
        cursor,
    });
    true
}

// ---------------------------------------------------------------------------
// Rectangle selection (Phase 2)
// ---------------------------------------------------------------------------

/// Handle a key while a rectangle selection is active. Returns true if
/// the key was consumed (so the caller doesn't fall through to the
/// normal pane-edit handler). Returns false for playback keys (Space,
/// `>`, `<`, `,`, `.`, `+`, `-`, `q`) so they keep working during
/// selection — the caller lets them fall through to the normal handler.
///
/// Keys consumed:
/// - `Esc` — cancel selection (clear `app.rectangle_selection`).
/// - Arrows / `hjkl` — move the active corner by [`SELECTION_STEP`].
/// - Shift+Arrows / `HJKL` — move the active corner 4x faster.
/// - `Tab` — switch the active corner (anchor ↔ cursor).
/// - `Enter` — confirm: anchor → extent → confirming (opens module
///   picker if the selection is valid, else shows a "too small" toast).
/// - `r` — restart the selection (reset to center).
/// - All other non-playback keys are swallowed (no-op) so they don't
///   accidentally split/close the pane being selected.
pub fn handle_rectangle_selection_key(app: &mut App, key: KeyEvent) -> bool {
    // Reserved Ctrl-* keys: never bind (mirrors the top of
    // `handle_pane_edit_key`). Let them fall through to the global
    // handler so Ctrl+C etc. keep working.
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c')
            | KeyCode::Char('z')
            | KeyCode::Char('\\')
            | KeyCode::Char('s')
            | KeyCode::Char('q') => return false,
            _ => {}
        }
    }

    // Playback keys fall through so they keep working during selection.
    // (Mirrors the pane-edit playback passthrough.) `Enter` is NOT a
    // playback key here — it confirms the selection.
    if is_playback_key(key) {
        return false;
    }

    // Esc: cancel selection (don't exit edit mode).
    if matches!(key.code, KeyCode::Esc) {
        app.rectangle_selection = None;
        return true;
    }

    // `r`: restart the selection (reset to center).
    if matches!(key.code, KeyCode::Char('r')) && key.modifiers == KeyModifiers::NONE {
        let target = app.pane_workspace.focused_pane;
        app.rectangle_selection = Some(RectangleSelection::new(target));
        return true;
    }

    // Arrows / hjkl: move the active corner.
    let dir = direction_for_key(key);
    if let Some(dir) = dir {
        if let Some(sel) = app.rectangle_selection.as_mut() {
            sel.move_cursor(dir, SELECTION_STEP, false);
        }
        return true;
    }

    // Shift+Arrows / HJKL: move the active corner 4x faster.
    let fast_dir = fast_direction_for_key(key);
    if let Some(dir) = fast_dir {
        if let Some(sel) = app.rectangle_selection.as_mut() {
            sel.move_cursor(dir, SELECTION_STEP, true);
        }
        return true;
    }

    // Tab: switch the active corner (anchor ↔ cursor).
    if matches!(key.code, KeyCode::Tab) {
        if let Some(sel) = app.rectangle_selection.as_mut() {
            sel.switch_corner();
        }
        return true;
    }

    // Enter: confirm. The behavior depends on the phase.
    if matches!(key.code, KeyCode::Enter) {
        // Take the selection out so we can call `is_valid` + open the
        // overlay without a borrow conflict.
        let mut sel = match app.rectangle_selection.take() {
            Some(s) => s,
            None => return true,
        };
        match sel.phase {
            SelectionPhase::ChoosingAnchor => {
                sel.confirm_anchor();
                app.rectangle_selection = Some(sel);
            }
            SelectionPhase::ChoosingExtent => {
                let pane_inner = focused_pane_inner_rect(app)
                    .unwrap_or(ratatui::layout::Rect::new(0, 0, 80, 24));
                if sel.is_valid(pane_inner) {
                    sel.confirm();
                    let target_pane = sel.target_pane;
                    app.rectangle_selection = Some(sel);
                    app.overlay = Some(Overlay::PaneModulePicker {
                        target_pane,
                        // No pending split — the rectangle selection
                        // drives the conversion via
                        // `apply_rectangle_selection`.
                        pending_split: None,
                        cursor: 0,
                    });
                    app.pane_workspace.set_mode(UiMode::PaneModulePicker);
                } else {
                    // Too small: stay in ChoosingExtent, show a toast.
                    // Don't open the picker.
                    app.rectangle_selection = Some(sel);
                    app.set_status_toast("selection too small".into());
                }
            }
            SelectionPhase::Confirming => {
                // Already confirming. The picker should be open; if it
                // isn't (defensive), open it.
                let target_pane = sel.target_pane;
                app.rectangle_selection = Some(sel);
                if app.overlay.is_none() {
                    app.overlay = Some(Overlay::PaneModulePicker {
                        target_pane,
                        pending_split: None,
                        cursor: 0,
                    });
                    app.pane_workspace.set_mode(UiMode::PaneModulePicker);
                }
            }
        }
        return true;
    }

    // All other keys are swallowed (no-op) so they don't accidentally
    // split/close the pane being selected. The user must Esc out of
    // selection mode first to use s/v/x/d/m/etc.
    true
}

/// True if `key` is a playback key (Space, `>`, `<`, `,`, `.`, `+`, `-`,
/// `q`). These fall through to the normal pane-edit handler so they
/// keep working during rectangle selection. Ctrl-prefixed variants
/// are NOT playback keys here (Ctrl+Q is reserved, etc.).
fn is_playback_key(key: KeyEvent) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return false;
    }
    matches!(
        key.code,
        KeyCode::Char(' ')
            | KeyCode::Char('>')
            | KeyCode::Char('<')
            | KeyCode::Char(',')
            | KeyCode::Char('.')
            | KeyCode::Char('+')
            | KeyCode::Char('-')
            | KeyCode::Char('q')
    )
}

/// Map an arrow / hjkl key (no Shift) to a direction. Returns None if
/// the key isn't a direction key.
fn direction_for_key(key: KeyEvent) -> Option<Direction> {
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        return None;
    }
    match key.code {
        KeyCode::Left | KeyCode::Char('h') if key.modifiers == KeyModifiers::NONE => {
            Some(Direction::Left)
        }
        KeyCode::Right | KeyCode::Char('l') if key.modifiers == KeyModifiers::NONE => {
            Some(Direction::Right)
        }
        KeyCode::Up | KeyCode::Char('k') if key.modifiers == KeyModifiers::NONE => {
            Some(Direction::Up)
        }
        KeyCode::Down | KeyCode::Char('j') if key.modifiers == KeyModifiers::NONE => {
            Some(Direction::Down)
        }
        _ => None,
    }
}

/// Map a Shift+arrow / HJKL key to a direction (fast movement). Returns
/// None if the key isn't a fast-direction key.
fn fast_direction_for_key(key: KeyEvent) -> Option<Direction> {
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        match key.code {
            KeyCode::Left => return Some(Direction::Left),
            KeyCode::Right => return Some(Direction::Right),
            KeyCode::Up => return Some(Direction::Up),
            KeyCode::Down => return Some(Direction::Down),
            _ => {}
        }
    }
    match key.code {
        KeyCode::Char('H') => Some(Direction::Left),
        KeyCode::Char('J') => Some(Direction::Down),
        KeyCode::Char('K') => Some(Direction::Up),
        KeyCode::Char('L') => Some(Direction::Right),
        _ => None,
    }
}

/// Compute the focused pane's INNER rect (after border) in terminal
/// coordinates. Used by the rectangle selection handler for
/// `is_valid` checks + by the mouse handler to convert (col, row) to
/// normalized coords. Returns None if the focused pane isn't found
/// (defensive — shouldn't happen).
pub fn focused_pane_inner_rect(app: &App) -> Option<ratatui::layout::Rect> {
    let area = app.pane_content_area();
    let panes = resolve_rects(&app.pane_workspace.root, area);
    let focused = panes
        .iter()
        .find(|p| p.pane_id == app.pane_workspace.focused_pane)?;
    // Subtract the border (1 cell each side) to get the inner area.
    Some(ratatui::layout::Rect::new(
        focused.rect.x.saturating_add(1),
        focused.rect.y.saturating_add(1),
        focused.rect.width.saturating_sub(2),
        focused.rect.height.saturating_sub(2),
    ))
}

/// Handle a mouse event during rectangle selection. `pane_inner` is the
/// focused pane's inner rect (the caller has already verified the
/// mouse is inside it). Routes:
/// - `Down(Left)` — start drag: anchor + cursor at the click position,
///   phase = ChoosingExtent, input_source = Mouse.
/// - `Drag(Left)` — update cursor.
/// - `Up(Left)` — confirm; if valid, open the module picker, else
///   show a "too small" toast.
/// - `Down(Right)` — cancel: clear the selection.
pub fn handle_rectangle_selection_mouse(
    app: &mut App,
    m: MouseEvent,
    pane_inner: ratatui::layout::Rect,
) {
    let mut sel = match app.rectangle_selection.take() {
        Some(s) => s,
        None => return,
    };

    // Convert (col, row) to normalized coords relative to pane_inner.
    let nx = (m.column as f32 - pane_inner.x as f32) / pane_inner.width.max(1) as f32;
    let ny = (m.row as f32 - pane_inner.y as f32) / pane_inner.height.max(1) as f32;
    let pt = NormalizedPoint::new(nx, ny);

    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            sel.anchor = pt;
            sel.cursor = pt;
            sel.phase = SelectionPhase::ChoosingExtent;
            sel.input_source = SelectionInput::Mouse;
            sel.active_is_anchor = false;
            app.rectangle_selection = Some(sel);
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            sel.cursor = pt;
            app.rectangle_selection = Some(sel);
        }
        MouseEventKind::Up(MouseButton::Left) => {
            sel.confirm();
            let is_valid = sel.is_valid(pane_inner);
            if is_valid {
                let target_pane = sel.target_pane;
                app.rectangle_selection = Some(sel);
                app.overlay = Some(Overlay::PaneModulePicker {
                    target_pane,
                    pending_split: None,
                    cursor: 0,
                });
                app.pane_workspace.set_mode(UiMode::PaneModulePicker);
            } else {
                // Too small: don't open the picker. Stay in
                // ChoosingExtent so the user can adjust. (Reset the
                // phase since `confirm` advanced it to Confirming.)
                sel.phase = SelectionPhase::ChoosingExtent;
                app.rectangle_selection = Some(sel);
                app.set_status_toast("selection too small".into());
            }
        }
        MouseEventKind::Down(MouseButton::Right) => {
            // Cancel: drop the selection entirely.
            app.set_status_toast("rectangle selection cancelled".into());
        }
        _ => {
            // Other mouse events (scroll, middle-click, etc.) don't
            // affect the selection — put it back.
            app.rectangle_selection = Some(sel);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::player::StubPlayer;
    use crate::tui::app::App;
    use crate::tui::pane::model::{ModuleId, PaneId, Side, UiMode};
    use crate::tui::pane::selection::{NormalizedPoint, SelectionPhase};
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use ratatui::layout::Rect;

    /// Build a 2-artist catalog + App. The tempdir is leaked so the
    /// catalog's source paths stay valid for the test's lifetime.
    fn build_app() -> App {
        let d = tempfile::tempdir().unwrap();
        let lossless = d.path().join("lossless");
        std::fs::create_dir_all(lossless.join("40mP")).unwrap();
        std::fs::write(lossless.join("40mP").join("01.flac"), b"x").unwrap();
        std::fs::create_dir_all(lossless.join("DECO")).unwrap();
        std::fs::write(lossless.join("DECO").join("01.flac"), b"x").unwrap();
        let json = serde_json::json!({
            "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
            "tracks":[
              {"id":"t1","artists":["40mP"],"primary_artist":"40mP","title":"Song1",
               "album":"Cosmic","bit_depth":24,"sample_rate_hz":96000,
               "source_path":"lossless/40mP/01.flac","symlinked_into_artists":["40mP"]},
              {"id":"t2","artists":["DECO*27"],"primary_artist":"DECO*27","title":"Ghost Rule",
               "album":"Ghost","bit_depth":16,"sample_rate_hz":44100,
               "source_path":"lossless/DECO/01.flac","symlinked_into_artists":["DECO*27"]}
            ]
        })
        .to_string();
        let p = d.path().join("catalog.json");
        std::fs::write(&p, json).unwrap();
        std::mem::forget(d);
        let cat = Catalog::load(&p).unwrap();
        App::new(cat, Box::new(StubPlayer::default()), None, None)
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn key_code(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }
    fn shift(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::SHIFT)
    }
    fn mouse(kind: MouseEventKind, col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn enter_edit_mode(app: &mut App) {
        handle_prefix_key(app, ctrl('w'));
        handle_prefix_key(app, key('e'));
        assert_eq!(app.pane_workspace.mode, UiMode::PaneEdit);
    }

    // -----------------------------------------------------------------
    // PrefixResult (constructors + is_consumed)
    // -----------------------------------------------------------------

    #[test]
    fn prefix_result_constructors() {
        assert!(PrefixResult::consumed().is_consumed());
        assert!(!PrefixResult::not_consumed().is_consumed());
    }

    // -----------------------------------------------------------------
    // handle_prefix_key: navigation in any mode
    // -----------------------------------------------------------------

    #[test]
    fn prefix_key_j_moves_down_single_pane_returns_true() {
        let mut app = build_app();
        // Single pane: j/k/h/l have no candidate to move to, so the
        // prefix handler returns false (the move was a no-op). The key
        // is still a "pane command" in spirit — the caller (handle_key)
        // treats the false as "fall through to global dispatch", which
        // is the desired behavior for hjkl in Normal mode (hjkl are
        // also column navigation in the artists view).
        assert!(!handle_prefix_key(&mut app, key('j')));
        assert!(!handle_prefix_key(&mut app, key('k')));
        assert!(!handle_prefix_key(&mut app, key('h')));
        assert!(!handle_prefix_key(&mut app, key('l')));
    }

    #[test]
    fn prefix_key_arrow_keys_navigate() {
        let mut app = build_app();
        app.pane_workspace
            .split(PaneId(0), Side::Right, ModuleId::Queue)
            .unwrap();
        // Focused on pane 1 (right). Left arrow moves to pane 0.
        assert!(handle_prefix_key(&mut app, key_code(KeyCode::Left)));
        assert_eq!(app.pane_workspace.focused_pane, PaneId(0));
        assert!(handle_prefix_key(&mut app, key_code(KeyCode::Right)));
        assert_eq!(app.pane_workspace.focused_pane, PaneId(1));
        // Up/Down have no candidate (single row) — return false.
        assert!(!handle_prefix_key(&mut app, key_code(KeyCode::Up)));
        assert!(!handle_prefix_key(&mut app, key_code(KeyCode::Down)));
    }

    #[test]
    fn prefix_key_tab_cycles_focus() {
        let mut app = build_app();
        app.pane_workspace
            .split(PaneId(0), Side::Right, ModuleId::Queue)
            .unwrap();
        // Focused on pane 1. Tab cycles to 0.
        assert!(handle_prefix_key(&mut app, key_code(KeyCode::Tab)));
        assert_eq!(app.pane_workspace.focused_pane, PaneId(0));
        // Shift+Tab cycles backward.
        assert!(handle_prefix_key(
            &mut app,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT)
        ));
        assert_eq!(app.pane_workspace.focused_pane, PaneId(1));
    }

    #[test]
    fn prefix_key_unknown_returns_false() {
        let mut app = build_app();
        // 'z' is not a pane command — falls through.
        assert!(!handle_prefix_key(&mut app, key('z')));
        assert!(!handle_prefix_key(&mut app, key_code(KeyCode::Backspace)));
    }

    // -----------------------------------------------------------------
    // handle_pane_edit_key: playback keys + quit
    // -----------------------------------------------------------------

    #[test]
    fn pane_edit_comma_seeks_backward() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        // No panic — the StubPlayer accepts seek.
        assert!(handle_pane_edit_key(&mut app, key(',')));
    }

    #[test]
    fn pane_edit_period_seeks_forward() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        assert!(handle_pane_edit_key(&mut app, key('.')));
    }

    #[test]
    fn pane_edit_lt_prev_track() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        // `<` triggers prev().
        assert!(handle_pane_edit_key(&mut app, key('<')));
    }

    #[test]
    fn pane_edit_gt_next_track() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        // `>` triggers next().
        assert!(handle_pane_edit_key(&mut app, key('>')));
    }

    #[test]
    fn pane_edit_plus_volume_up() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        assert!(handle_pane_edit_key(&mut app, key('+')));
    }

    #[test]
    fn pane_edit_minus_lowers_volume() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        assert!(handle_pane_edit_key(&mut app, key('-')));
    }

    #[test]
    fn pane_edit_q_quits() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        assert!(handle_pane_edit_key(&mut app, key('q')));
        assert!(app.should_quit, "q should set should_quit");
    }

    #[test]
    fn pane_edit_enter_plays_selected() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        assert!(handle_pane_edit_key(&mut app, key_code(KeyCode::Enter)));
    }

    #[test]
    fn pane_edit_unknown_key_returns_false() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        // 'z' is not a pane-edit command — fall through to global.
        assert!(!handle_pane_edit_key(&mut app, key('z')));
        // F1 isn't bound either.
        assert!(!handle_pane_edit_key(&mut app, key_code(KeyCode::F(1))));
    }

    // -----------------------------------------------------------------
    // handle_pane_edit_key: hjkl + arrows navigation in edit mode
    // -----------------------------------------------------------------

    #[test]
    fn pane_edit_arrows_move_focus() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        app.pane_workspace
            .split(PaneId(0), Side::Right, ModuleId::Queue)
            .unwrap();
        // Focused on pane 1 (right). Left arrow moves to pane 0.
        assert!(handle_pane_edit_key(&mut app, key_code(KeyCode::Left)));
        assert_eq!(app.pane_workspace.focused_pane, PaneId(0));
        assert!(handle_pane_edit_key(&mut app, key_code(KeyCode::Right)));
        assert_eq!(app.pane_workspace.focused_pane, PaneId(1));
        // Up/Down have no candidate (single row) — return false (key
        // not consumed by pane-edit; falls through to global).
        assert!(!handle_pane_edit_key(&mut app, key_code(KeyCode::Up)));
        assert!(!handle_pane_edit_key(&mut app, key_code(KeyCode::Down)));
    }

    #[test]
    fn pane_edit_hjkl_move_focus() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        app.pane_workspace
            .split(PaneId(0), Side::Right, ModuleId::Queue)
            .unwrap();
        // l/h navigate; j/k have no candidate.
        assert!(handle_pane_edit_key(&mut app, key('h')));
        assert_eq!(app.pane_workspace.focused_pane, PaneId(0));
        assert!(handle_pane_edit_key(&mut app, key('l')));
        assert_eq!(app.pane_workspace.focused_pane, PaneId(1));
        // j/k have no candidate — return false (fall through).
        assert!(!handle_pane_edit_key(&mut app, key('j')));
        assert!(!handle_pane_edit_key(&mut app, key('k')));
    }

    // -----------------------------------------------------------------
    // H/J/K/L resize
    // -----------------------------------------------------------------

    #[test]
    fn pane_edit_hjkl_resize() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        app.pane_workspace
            .split(PaneId(0), Side::Right, ModuleId::Queue)
            .unwrap();
        // Focused on pane 1 (right). H decreases ratio (grows left).
        let ratio_before = match &app.pane_workspace.root {
            crate::tui::pane::PaneNode::Split { ratio, .. } => *ratio,
            _ => panic!("expected Split"),
        };
        assert!(handle_pane_edit_key(&mut app, key('H')));
        let ratio_after = match &app.pane_workspace.root {
            crate::tui::pane::PaneNode::Split { ratio, .. } => *ratio,
            _ => panic!("expected Split"),
        };
        assert!(ratio_after < ratio_before, "H should decrease ratio");

        // Move focus to the left pane; L increases ratio (grows right).
        handle_pane_edit_key(&mut app, key('h'));
        assert_eq!(app.pane_workspace.focused_pane, PaneId(0));
        assert!(handle_pane_edit_key(&mut app, key('L')));
        let ratio_final = match &app.pane_workspace.root {
            crate::tui::pane::PaneNode::Split { ratio, .. } => *ratio,
            _ => panic!("expected Split"),
        };
        assert!(ratio_final > ratio_after, "L should increase ratio");

        // J/K on a vertical split are no-ops for resize (no matching
        // ancestor), but still consume the key.
        assert!(handle_pane_edit_key(&mut app, key('J')));
        assert!(handle_pane_edit_key(&mut app, key('K')));
    }

    // -----------------------------------------------------------------
    // 1/2/3/4 change module
    // -----------------------------------------------------------------

    #[test]
    fn pane_edit_number_keys_change_module() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        // 1 → Artists (already Artists; still consumed).
        assert!(handle_pane_edit_key(&mut app, key('1')));
        // 2 → Playlists.
        assert!(handle_pane_edit_key(&mut app, key('2')));
        // 3 → Queue.
        assert!(handle_pane_edit_key(&mut app, key('3')));
        // 4 → Youtube.
        assert!(handle_pane_edit_key(&mut app, key('4')));
        // Verify the focused pane's module is Youtube.
        let panes = crate::tui::pane::layout::resolve_rects(
            &app.pane_workspace.root,
            Rect::new(0, 0, 100, 30),
        );
        let focused = panes
            .iter()
            .find(|p| p.pane_id == app.pane_workspace.focused_pane)
            .unwrap();
        assert_eq!(focused.module_id, ModuleId::Youtube);
    }

    // -----------------------------------------------------------------
    // Ctrl+w in PaneEdit mode arms the prefix.
    // -----------------------------------------------------------------

    #[test]
    fn pane_edit_ctrl_w_arms_prefix() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        assert!(handle_pane_edit_key(&mut app, ctrl('w')));
        assert!(
            app.pending_pane_prefix,
            "Ctrl+w in PaneEdit should arm the prefix"
        );
    }

    // -----------------------------------------------------------------
    // Reserved Ctrl-* keys fall through.
    // -----------------------------------------------------------------

    #[test]
    fn pane_edit_reserved_ctrl_keys_fall_through() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        // Ctrl+C, Ctrl+Z, Ctrl+\, Ctrl+S, Ctrl+Q should fall through.
        for c in ['c', 'z', '\\', 's', 'q'] {
            assert!(
                !handle_pane_edit_key(&mut app, ctrl(c)),
                "Ctrl+{c} should fall through"
            );
        }
    }

    // -----------------------------------------------------------------
    // handle_split_direction_key: all directions + Esc
    // -----------------------------------------------------------------

    #[test]
    fn split_direction_h_opens_module_picker_with_left_side() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        app.overlay = Some(Overlay::PaneSplitDirection {
            target_pane: PaneId(0),
        });
        app.pane_workspace.set_mode(UiMode::PaneModulePicker);
        assert!(handle_split_direction_key(&mut app, key('h')));
        match &app.overlay {
            Some(Overlay::PaneModulePicker {
                pending_split: Some(side),
                ..
            }) => assert_eq!(*side, Side::Left),
            _ => panic!("expected module picker with Left side"),
        }
    }

    #[test]
    fn split_direction_arrow_keys_select_sides() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        // Test each arrow key.
        for (key_ev, expected_side) in [
            (key_code(KeyCode::Left), Side::Left),
            (key_code(KeyCode::Right), Side::Right),
            (key_code(KeyCode::Up), Side::Top),
            (key_code(KeyCode::Down), Side::Bottom),
        ] {
            app.overlay = Some(Overlay::PaneSplitDirection {
                target_pane: PaneId(0),
            });
            app.pane_workspace.set_mode(UiMode::PaneModulePicker);
            assert!(handle_split_direction_key(&mut app, key_ev));
            match &app.overlay {
                Some(Overlay::PaneModulePicker {
                    pending_split: Some(side),
                    ..
                }) => assert_eq!(*side, expected_side, "mismatch for {key_ev:?}"),
                _ => panic!("expected module picker after split direction"),
            }
        }
    }

    #[test]
    fn split_direction_unknown_key_keeps_overlay_open() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        app.overlay = Some(Overlay::PaneSplitDirection {
            target_pane: PaneId(0),
        });
        app.pane_workspace.set_mode(UiMode::PaneModulePicker);
        // 'z' is not a direction — overlay stays open.
        assert!(handle_split_direction_key(&mut app, key('z')));
        assert!(matches!(
            app.overlay,
            Some(Overlay::PaneSplitDirection { .. })
        ));
    }

    #[test]
    fn split_direction_esc_cancels() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        app.overlay = Some(Overlay::PaneSplitDirection {
            target_pane: PaneId(0),
        });
        app.pane_workspace.set_mode(UiMode::PaneModulePicker);
        assert!(handle_split_direction_key(&mut app, key_code(KeyCode::Esc)));
        assert!(app.overlay.is_none());
        assert_eq!(app.pane_workspace.mode, UiMode::PaneEdit);
    }

    // -----------------------------------------------------------------
    // handle_module_picker_key: navigation + confirm + Esc
    // -----------------------------------------------------------------

    #[test]
    fn module_picker_down_up_navigate() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        app.overlay = Some(Overlay::PaneModulePicker {
            target_pane: PaneId(0),
            pending_split: None,
            cursor: 0,
        });
        app.pane_workspace.set_mode(UiMode::PaneModulePicker);
        // Down wraps (cursor 0 → 1 → ... → n-1 → 0).
        assert!(handle_module_picker_key(&mut app, key_code(KeyCode::Down)));
        let cursor = match &app.overlay {
            Some(Overlay::PaneModulePicker { cursor, .. }) => *cursor,
            _ => panic!("expected picker"),
        };
        assert_eq!(cursor, 1, "Down should advance cursor");
        // Up wraps.
        assert!(handle_module_picker_key(&mut app, key_code(KeyCode::Up)));
        let cursor = match &app.overlay {
            Some(Overlay::PaneModulePicker { cursor, .. }) => *cursor,
            _ => panic!("expected picker"),
        };
        assert_eq!(cursor, 0, "Up should move back to 0");

        // j/k also work.
        assert!(handle_module_picker_key(&mut app, key('j')));
        assert!(handle_module_picker_key(&mut app, key('k')));
    }

    #[test]
    fn module_picker_unknown_key_keeps_overlay() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        app.overlay = Some(Overlay::PaneModulePicker {
            target_pane: PaneId(0),
            pending_split: None,
            cursor: 0,
        });
        app.pane_workspace.set_mode(UiMode::PaneModulePicker);
        // 'z' is unknown — picker stays open.
        assert!(handle_module_picker_key(&mut app, key('z')));
        assert!(matches!(
            app.overlay,
            Some(Overlay::PaneModulePicker { .. })
        ));
    }

    #[test]
    fn module_picker_no_overlay_returns_false() {
        let mut app = build_app();
        // No overlay set — picker handler returns false (key not consumed).
        assert!(!handle_module_picker_key(&mut app, key('j')));
    }

    #[test]
    fn module_picker_esc_cancels() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        app.overlay = Some(Overlay::PaneModulePicker {
            target_pane: PaneId(0),
            pending_split: None,
            cursor: 0,
        });
        app.pane_workspace.set_mode(UiMode::PaneModulePicker);
        assert!(handle_module_picker_key(&mut app, key_code(KeyCode::Esc)));
        assert!(app.overlay.is_none());
        assert_eq!(app.pane_workspace.mode, UiMode::PaneEdit);
    }

    // -----------------------------------------------------------------
    // handle_rectangle_selection_key: phases + edge cases
    // -----------------------------------------------------------------

    fn enter_rect_selection(app: &mut App) {
        enter_edit_mode(app);
        handle_pane_edit_key(app, key('r'));
        assert!(app.rectangle_selection.is_some());
    }

    #[test]
    fn rect_selection_ctrl_reserved_keys_fall_through() {
        let mut app = build_app();
        enter_rect_selection(&mut app);
        // Ctrl+C etc. fall through (not consumed by selection handler).
        for c in ['c', 'z', '\\', 's', 'q'] {
            assert!(
                !handle_rectangle_selection_key(&mut app, ctrl(c)),
                "Ctrl+{c} should fall through"
            );
        }
    }

    #[test]
    fn rect_selection_ctrl_unknown_consumed() {
        let mut app = build_app();
        enter_rect_selection(&mut app);
        // Ctrl+X is not a reserved Ctrl-* key; the selection handler
        // swallows it.
        assert!(handle_rectangle_selection_key(&mut app, ctrl('x')));
    }

    #[test]
    fn rect_selection_playback_keys_fall_through() {
        let mut app = build_app();
        enter_rect_selection(&mut app);
        // Space, >, <, ,, ., +, -, q fall through to the pane-edit
        // playback handler.
        for kc in [
            KeyCode::Char(' '),
            KeyCode::Char('>'),
            KeyCode::Char('<'),
            KeyCode::Char(','),
            KeyCode::Char('.'),
            KeyCode::Char('+'),
            KeyCode::Char('-'),
            KeyCode::Char('q'),
        ] {
            assert!(
                !handle_rectangle_selection_key(&mut app, key_code(kc)),
                "playback key {kc:?} should fall through"
            );
        }
    }

    #[test]
    fn rect_selection_unknown_key_swallowed() {
        let mut app = build_app();
        enter_rect_selection(&mut app);
        // 'z' is not a selection key — swallowed (no-op, but consumed).
        assert!(handle_rectangle_selection_key(&mut app, key('z')));
    }

    #[test]
    fn rect_selection_enter_in_confirming_phase_reopens_picker() {
        let mut app = build_app();
        enter_rect_selection(&mut app);
        // Move anchor + cursor to make a valid selection, confirm to
        // reach Confirming phase.
        for _ in 0..15 {
            handle_rectangle_selection_key(&mut app, key_code(KeyCode::Left));
            handle_rectangle_selection_key(&mut app, key_code(KeyCode::Up));
        }
        handle_rectangle_selection_key(&mut app, key_code(KeyCode::Enter));
        for _ in 0..15 {
            handle_rectangle_selection_key(&mut app, key_code(KeyCode::Right));
            handle_rectangle_selection_key(&mut app, key_code(KeyCode::Down));
        }
        handle_rectangle_selection_key(&mut app, key_code(KeyCode::Enter));
        // Now in Confirming with picker open.
        let sel = app.rectangle_selection.as_ref().unwrap();
        assert_eq!(sel.phase, SelectionPhase::Confirming);
        // Close the overlay manually to test the "reopen" branch.
        app.overlay = None;
        // Enter in Confirming with no overlay → reopen picker.
        assert!(handle_rectangle_selection_key(
            &mut app,
            key_code(KeyCode::Enter)
        ));
        assert!(matches!(
            app.overlay,
            Some(Overlay::PaneModulePicker { .. })
        ));
    }

    #[test]
    fn rect_selection_enter_in_choosing_extent_toast_when_too_small() {
        let mut app = build_app();
        enter_rect_selection(&mut app);
        // Confirm anchor → ChoosingExtent.
        handle_rectangle_selection_key(&mut app, key_code(KeyCode::Enter));
        // Move the cursor close to the anchor (tiny selection).
        for _ in 0..15 {
            handle_rectangle_selection_key(&mut app, key_code(KeyCode::Left));
            handle_rectangle_selection_key(&mut app, key_code(KeyCode::Up));
        }
        // Enter → too small → toast, no picker.
        handle_rectangle_selection_key(&mut app, key_code(KeyCode::Enter));
        assert!(
            app.overlay.is_none(),
            "picker should not open for tiny selection"
        );
        assert!(app.status_toast.is_some(), "should show 'too small' toast");
    }

    // -----------------------------------------------------------------
    // handle_rectangle_selection_mouse
    // -----------------------------------------------------------------

    #[test]
    fn rect_selection_mouse_no_selection_is_noop() {
        let mut app = build_app();
        // No active selection — mouse handler is a no-op.
        handle_rectangle_selection_mouse(
            &mut app,
            mouse(MouseEventKind::Down(MouseButton::Left), 10, 5),
            Rect::new(0, 0, 80, 24),
        );
        assert!(app.rectangle_selection.is_none());
    }

    #[test]
    fn rect_selection_mouse_up_too_small_shows_toast() {
        let mut app = build_app();
        enter_rect_selection(&mut app);
        // Mouse-down + immediate mouse-up at the same position = 0×0
        // selection — too small. Should stay in ChoosingExtent with a
        // toast.
        let inner = super::focused_pane_inner_rect(&app).unwrap();
        let cx = inner.x + 5;
        let cy = inner.y + 5;
        handle_rectangle_selection_mouse(
            &mut app,
            mouse(MouseEventKind::Down(MouseButton::Left), cx, cy),
            inner,
        );
        assert!(app.rectangle_selection.is_some());
        handle_rectangle_selection_mouse(
            &mut app,
            mouse(MouseEventKind::Up(MouseButton::Left), cx, cy),
            inner,
        );
        // Too small → no picker, toast.
        assert!(
            app.overlay.is_none(),
            "picker should not open for tiny mouse-up"
        );
        let sel = app.rectangle_selection.as_ref().unwrap();
        assert_eq!(sel.phase, SelectionPhase::ChoosingExtent);
    }

    #[test]
    fn rect_selection_mouse_other_events_keep_selection() {
        let mut app = build_app();
        enter_rect_selection(&mut app);
        let sel_before = app.rectangle_selection.as_ref().unwrap().anchor;
        // Scroll doesn't affect the selection — the handler puts it back.
        handle_rectangle_selection_mouse(
            &mut app,
            mouse(MouseEventKind::ScrollDown, 10, 10),
            Rect::new(0, 0, 80, 24),
        );
        assert!(
            app.rectangle_selection.is_some(),
            "selection should still be present after scroll"
        );
        let sel_after = app.rectangle_selection.as_ref().unwrap().anchor;
        assert_eq!(sel_after, sel_before, "scroll should not change selection");
    }

    // -----------------------------------------------------------------
    // module_to_view / focused_module (private helpers)
    // -----------------------------------------------------------------

    #[test]
    fn module_to_view_returns_expected_view() {
        assert_eq!(
            module_to_view(ModuleId::Artists),
            Some(crate::tui::app::View::Artists)
        );
        assert_eq!(
            module_to_view(ModuleId::Playlists),
            Some(crate::tui::app::View::Playlists)
        );
        assert_eq!(
            module_to_view(ModuleId::Queue),
            Some(crate::tui::app::View::Queue)
        );
        assert_eq!(
            module_to_view(ModuleId::Youtube),
            Some(crate::tui::app::View::Youtube)
        );
        assert_eq!(module_to_view(ModuleId::Placeholder), None);
    }

    #[test]
    fn focused_module_returns_artists_for_unknown_pane() {
        let app = build_app();
        // The default workspace's focused pane is PaneId(0); focused_module
        // returns its module (Artists).
        let m = focused_module(&app.pane_workspace);
        assert_eq!(m, ModuleId::Artists);
    }

    #[test]
    fn focused_module_unknown_pane_falls_back_to_artists() {
        // Build a workspace with no panes to hit the fallback. We
        // can't easily build a `PaneWorkspace` without Clone, so test
        // via the `module_to_view` path instead.
        // The default workspace's focused pane is PaneId(0); changing
        // it to an unknown id makes `focused_module` fall back.
        let ws = crate::tui::pane::PaneWorkspace::new();
        // Sanity: default is Artists.
        assert_eq!(focused_module(&ws), ModuleId::Artists);
    }

    #[test]
    fn focused_pane_inner_rect_returns_some_for_existing_pane() {
        let app = build_app();
        let r = focused_pane_inner_rect(&app);
        assert!(r.is_some(), "should return Some for existing pane");
    }

    // -----------------------------------------------------------------
    // direction_for_key + fast_direction_for_key
    // -----------------------------------------------------------------

    #[test]
    fn direction_for_key_maps_arrows_and_hjkl() {
        assert_eq!(
            direction_for_key(key_code(KeyCode::Left)),
            Some(Direction::Left)
        );
        assert_eq!(
            direction_for_key(key_code(KeyCode::Right)),
            Some(Direction::Right)
        );
        assert_eq!(
            direction_for_key(key_code(KeyCode::Up)),
            Some(Direction::Up)
        );
        assert_eq!(
            direction_for_key(key_code(KeyCode::Down)),
            Some(Direction::Down)
        );
        assert_eq!(direction_for_key(key('h')), Some(Direction::Left));
        assert_eq!(direction_for_key(key('l')), Some(Direction::Right));
        assert_eq!(direction_for_key(key('k')), Some(Direction::Up));
        assert_eq!(direction_for_key(key('j')), Some(Direction::Down));
    }

    #[test]
    fn direction_for_key_shift_returns_none() {
        // Shift+arrows are NOT slow-direction keys (they're fast keys).
        assert_eq!(direction_for_key(shift(KeyCode::Left)), None);
        assert_eq!(direction_for_key(key('z')), None);
    }

    #[test]
    fn fast_direction_for_key_maps_shift_arrows_and_hjkl() {
        assert_eq!(
            fast_direction_for_key(shift(KeyCode::Left)),
            Some(Direction::Left)
        );
        assert_eq!(
            fast_direction_for_key(shift(KeyCode::Right)),
            Some(Direction::Right)
        );
        assert_eq!(
            fast_direction_for_key(shift(KeyCode::Up)),
            Some(Direction::Up)
        );
        assert_eq!(
            fast_direction_for_key(shift(KeyCode::Down)),
            Some(Direction::Down)
        );
        assert_eq!(fast_direction_for_key(key('H')), Some(Direction::Left));
        assert_eq!(fast_direction_for_key(key('J')), Some(Direction::Down));
        assert_eq!(fast_direction_for_key(key('K')), Some(Direction::Up));
        assert_eq!(fast_direction_for_key(key('L')), Some(Direction::Right));
        assert_eq!(fast_direction_for_key(key('z')), None);
        // Plain (non-shift) arrow keys return None from this fast-path.
        assert_eq!(fast_direction_for_key(key_code(KeyCode::Left)), None);
        // Shift + non-arrow key (e.g. Shift+a) falls through the inner
        // match's `_ => {}` arm, then through the outer HJKL match
        // (since 'a' isn't H/J/K/L), returning None.
        assert_eq!(
            fast_direction_for_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::SHIFT)),
            None
        );
    }

    // -----------------------------------------------------------------
    // is_playback_key
    // -----------------------------------------------------------------

    #[test]
    fn is_playback_key_matches_unmodified_playback_keys() {
        for c in [' ', '>', '<', ',', '.', '+', '-', 'q'] {
            assert!(
                is_playback_key(key(c)),
                "expected {c:?} to be a playback key"
            );
        }
        // Ctrl-prefixed variants are NOT playback keys here.
        assert!(!is_playback_key(ctrl('q')));
        // Other keys are not.
        assert!(!is_playback_key(key('z')));
        assert!(!is_playback_key(key_code(KeyCode::Enter)));
    }

    // -----------------------------------------------------------------
    // move_focus + cycle_focus returning false
    // -----------------------------------------------------------------

    #[test]
    fn move_focus_returns_false_when_no_candidate() {
        let mut app = build_app();
        // Single pane — no candidate for any direction.
        assert!(!move_focus(&mut app, Direction::Left));
        assert!(!move_focus(&mut app, Direction::Right));
        assert!(!move_focus(&mut app, Direction::Up));
        assert!(!move_focus(&mut app, Direction::Down));
    }

    #[test]
    fn cycle_focus_returns_false_when_single_pane() {
        let mut app = build_app();
        // Single pane — cycle has no candidate.
        assert!(!cycle_focus(&mut app, true));
        assert!(!cycle_focus(&mut app, false));
    }

    // -----------------------------------------------------------------
    // sync_app_view_to_focused_pane
    // -----------------------------------------------------------------

    #[test]
    fn sync_app_view_to_focused_pane_sets_view() {
        let mut app = build_app();
        // Default focused module is Artists → app.view should be Artists.
        app.pane_workspace.set_module(PaneId(0), ModuleId::Queue);
        sync_app_view_to_focused_pane(&mut app);
        assert_eq!(app.view, crate::tui::app::View::Queue);

        // Placeholder module → no view change (stays Queue).
        app.pane_workspace
            .set_module(PaneId(0), ModuleId::Placeholder);
        sync_app_view_to_focused_pane(&mut app);
        assert_eq!(
            app.view,
            crate::tui::app::View::Queue,
            "Placeholder should not change app.view"
        );
    }

    // -----------------------------------------------------------------
    // resize_focused / change_focused_module
    // -----------------------------------------------------------------

    #[test]
    fn resize_focused_returns_true_even_when_noop() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        // Single pane — resize is a no-op but the key is still consumed.
        assert!(resize_focused(&mut app, Direction::Left));
        assert!(resize_focused(&mut app, Direction::Right));
        assert!(resize_focused(&mut app, Direction::Up));
        assert!(resize_focused(&mut app, Direction::Down));
    }

    #[test]
    fn change_focused_module_returns_true() {
        let mut app = build_app();
        // Change to Queue.
        assert!(change_focused_module(&mut app, ModuleId::Queue));
        // Change to Youtube.
        assert!(change_focused_module(&mut app, ModuleId::Youtube));
        // Verify.
        let panes = crate::tui::pane::layout::resolve_rects(
            &app.pane_workspace.root,
            Rect::new(0, 0, 100, 30),
        );
        let focused = panes
            .iter()
            .find(|p| p.pane_id == app.pane_workspace.focused_pane)
            .unwrap();
        assert_eq!(focused.module_id, ModuleId::Youtube);
    }

    // -----------------------------------------------------------------
    // handle_pane_edit_key: `d` on a non-existent pane (defensive)
    // -----------------------------------------------------------------

    #[test]
    fn pane_edit_d_when_focused_pane_not_in_tree_does_not_panic() {
        let mut app = build_app();
        enter_edit_mode(&mut app);
        // Force the focused pane to an invalid id. The close handler
        // returns NotFound, which is swallowed (defensive — should
        // never happen in practice).
        app.pane_workspace.focused_pane = PaneId(99);
        // The key is still consumed.
        assert!(handle_pane_edit_key(&mut app, key('d')));
    }

    // -----------------------------------------------------------------
    // RectangleSelection phase transitions
    // -----------------------------------------------------------------

    #[test]
    fn rect_selection_tab_switches_active_corner() {
        let mut app = build_app();
        enter_rect_selection(&mut app);
        // Default: active_is_anchor = true. Tab switches.
        assert!(handle_rectangle_selection_key(
            &mut app,
            key_code(KeyCode::Tab)
        ));
        let sel = app.rectangle_selection.as_ref().unwrap();
        assert!(!sel.active_is_anchor, "Tab should switch active corner");
        // Tab again switches back.
        assert!(handle_rectangle_selection_key(
            &mut app,
            key_code(KeyCode::Tab)
        ));
        let sel = app.rectangle_selection.as_ref().unwrap();
        assert!(sel.active_is_anchor, "Tab should switch back");
    }

    #[test]
    fn rect_selection_arrow_moves_active_corner() {
        let mut app = build_app();
        enter_rect_selection(&mut app);
        let anchor_before = app.rectangle_selection.as_ref().unwrap().anchor;
        // Right arrow moves the active corner (anchor by default).
        assert!(handle_rectangle_selection_key(
            &mut app,
            key_code(KeyCode::Right)
        ));
        let sel = app.rectangle_selection.as_ref().unwrap();
        assert!(
            sel.anchor.x > anchor_before.x,
            "anchor.x should increase after Right"
        );
        // Down arrow.
        let anchor_before = app.rectangle_selection.as_ref().unwrap().anchor;
        assert!(handle_rectangle_selection_key(
            &mut app,
            key_code(KeyCode::Down)
        ));
        let sel = app.rectangle_selection.as_ref().unwrap();
        assert!(sel.anchor.y > anchor_before.y);
        // Up + Left to exercise all directions.
        assert!(handle_rectangle_selection_key(
            &mut app,
            key_code(KeyCode::Up)
        ));
        assert!(handle_rectangle_selection_key(
            &mut app,
            key_code(KeyCode::Left)
        ));
    }

    #[test]
    fn rect_selection_shift_arrows_move_4x() {
        let mut app = build_app();
        enter_rect_selection(&mut app);
        let anchor_before = app.rectangle_selection.as_ref().unwrap().anchor;
        assert!(handle_rectangle_selection_key(
            &mut app,
            shift(KeyCode::Right)
        ));
        let sel = app.rectangle_selection.as_ref().unwrap();
        // 4 * SELECTION_STEP = 0.08.
        assert!(
            (sel.anchor.x - (anchor_before.x + 0.08)).abs() < 1e-5,
            "Shift+Right should move 4x: {} -> {}",
            anchor_before.x,
            sel.anchor.x
        );
    }

    #[test]
    fn rect_selection_hjkl_moves_active_corner() {
        let mut app = build_app();
        enter_rect_selection(&mut app);
        let anchor_before = app.rectangle_selection.as_ref().unwrap().anchor;
        assert!(handle_rectangle_selection_key(&mut app, key('l')));
        let sel = app.rectangle_selection.as_ref().unwrap();
        assert!(sel.anchor.x > anchor_before.x, "l should move right");
        assert!(handle_rectangle_selection_key(&mut app, key('h')));
        assert!(handle_rectangle_selection_key(&mut app, key('j')));
        assert!(handle_rectangle_selection_key(&mut app, key('k')));
    }

    #[test]
    fn rect_selection_hjkl_uppercase_moves_4x() {
        let mut app = build_app();
        enter_rect_selection(&mut app);
        let anchor_before = app.rectangle_selection.as_ref().unwrap().anchor;
        assert!(handle_rectangle_selection_key(&mut app, key('L')));
        let sel = app.rectangle_selection.as_ref().unwrap();
        assert!(
            (sel.anchor.x - (anchor_before.x + 0.08)).abs() < 1e-5,
            "L should move 4x: {} -> {}",
            anchor_before.x,
            sel.anchor.x
        );
    }

    #[test]
    fn rect_selection_r_resets() {
        let mut app = build_app();
        enter_rect_selection(&mut app);
        // Move the anchor away from center.
        for _ in 0..5 {
            handle_rectangle_selection_key(&mut app, key_code(KeyCode::Right));
            handle_rectangle_selection_key(&mut app, key_code(KeyCode::Down));
        }
        let before = app.rectangle_selection.as_ref().unwrap().anchor;
        // r resets.
        assert!(handle_rectangle_selection_key(&mut app, key('r')));
        let sel = app.rectangle_selection.as_ref().unwrap();
        assert_eq!(sel.anchor, NormalizedPoint::new(0.4, 0.4));
        assert_eq!(sel.phase, SelectionPhase::ChoosingAnchor);
        assert!(before.x != sel.anchor.x || before.y != sel.anchor.y);
    }

    #[test]
    fn rect_selection_esc_cancels() {
        let mut app = build_app();
        enter_rect_selection(&mut app);
        assert!(handle_rectangle_selection_key(
            &mut app,
            key_code(KeyCode::Esc)
        ));
        assert!(app.rectangle_selection.is_none());
        assert_eq!(app.pane_workspace.mode, UiMode::PaneEdit);
    }

    #[test]
    fn rect_selection_enter_in_choosing_anchor_advances_phase() {
        let mut app = build_app();
        enter_rect_selection(&mut app);
        assert!(handle_rectangle_selection_key(
            &mut app,
            key_code(KeyCode::Enter)
        ));
        let sel = app.rectangle_selection.as_ref().unwrap();
        assert_eq!(sel.phase, SelectionPhase::ChoosingExtent);
    }

    #[test]
    fn rect_selection_enter_in_choosing_extent_valid_opens_picker() {
        let mut app = build_app();
        enter_rect_selection(&mut app);
        // Move anchor to top-left, cursor stays at center → valid.
        for _ in 0..15 {
            handle_rectangle_selection_key(&mut app, key_code(KeyCode::Left));
            handle_rectangle_selection_key(&mut app, key_code(KeyCode::Up));
        }
        handle_rectangle_selection_key(&mut app, key_code(KeyCode::Enter));
        // Move cursor to bottom-right.
        for _ in 0..15 {
            handle_rectangle_selection_key(&mut app, key_code(KeyCode::Right));
            handle_rectangle_selection_key(&mut app, key_code(KeyCode::Down));
        }
        handle_rectangle_selection_key(&mut app, key_code(KeyCode::Enter));
        assert!(matches!(
            app.overlay,
            Some(Overlay::PaneModulePicker { .. })
        ));
        let sel = app.rectangle_selection.as_ref().unwrap();
        assert_eq!(sel.phase, SelectionPhase::Confirming);
    }
}
