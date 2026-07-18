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
