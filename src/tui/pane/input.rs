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

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::app::{App, Overlay};
use crate::tui::pane::focus::move_focus_directional;
use crate::tui::pane::layout::resolve_rects;
use crate::tui::pane::model::{Direction, ModuleId, Side, UiMode};
use crate::tui::pane::{PaneWorkspace, RESIZE_STEP};

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

    // Esc: exit pane edit mode.
    if matches!(key.code, KeyCode::Esc) {
        app.pane_workspace.exit_edit_mode();
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
            if let Some(side) = pending_split {
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
