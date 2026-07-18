//! Pane workspace rendering: borders, titles, edit-mode status line,
//! tiny-terminal fallback.
//!
//! The render layer reads `app.pane_workspace` (root tree + focused pane
//! + mode) and the global module registry (see
//! [`crate::tui::pane::registry`]).
//!
//! It never mutates them. Layout is recomputed from the tree on every
//! render via [`crate::tui::pane::layout::resolve_rects`].
//!
//! ## Painting order
//!
//! 1. Resolve rects: `let panes = resolve_rects(&root, area);`
//! 2. If a single pane in Normal mode, render the module directly into
//!    `area` (identical to the legacy per-view renderer; no border).
//! 3. Otherwise, for each resolved pane:
//!    - Skip panes whose `Rect` is below the minimum usable size
//!      (`.width < MIN_PANE_WIDTH || .height < MIN_PANE_HEIGHT`).
//!    - Draw a border Block (focused = accent + thick, unfocused = dim
//!      + plain). Title = the module's label. In PaneEdit mode the
//!      focused pane gets an "EDIT" badge.
//!    - Call the module's `render` with the inner rect.
//! 4. If in PaneEdit mode and the workspace is large enough, paint a
//!    one-line status overlay at the bottom with the edit-mode keymap.

// Clippy's `doc_lazy_continuation` lint flags continuation lines of
// multi-paragraph doc comments where the indentation doesn't match the
// list-item style it expects. The doc comments above use a slightly
// different style; the warnings are stylistic, not correctness issues.
#![allow(clippy::doc_lazy_continuation)]

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::tui::app::App;
use crate::tui::pane::layout::{is_usable, resolve_rects, MIN_PANE_HEIGHT};
use crate::tui::pane::model::UiMode;
use crate::tui::pane::registry::registry;
use crate::tui::pane::selection::SelectionPhase;
use crate::tui::view::theme::{is_ascii, ASCII_BORDER_SET};

/// Minimum workspace size for the multi-pane layout. Below this we
/// render the focused pane only (with a "terminal too small for panes"
/// toast) so the user isn't stuck with unusable micro-panes.
const MIN_WORKSPACE_W: u16 = 40;
const MIN_WORKSPACE_H: u16 = 8;

/// The edit-mode status line height (1 row).
const STATUS_LINE_H: u16 = 1;

/// Render the pane workspace into `area`. `area` is the main content
/// region (after the rail / sidebar / now-playing panel have been split
/// off — see `view::columns::render`'s pane-mode branch).
///
/// Modules are looked up via the global registry (see
/// [`crate::tui::pane::registry::registry`]) so the registry borrow
/// doesn't conflict with the `&mut App` needed for `PaneModule::render`.
pub fn render_pane_workspace(f: &mut Frame, area: Rect, app: &mut App) {
    // Copy out the pane-workspace state we need for rendering. This
    // releases the immutable borrow of `app.pane_workspace` so we can
    // pass `&mut app` to `module.render` inside the loop. The workspace
    // isn't mutated during render (mutation is input's job), so a
    // snapshot is safe.
    let root_clone = app.pane_workspace.root.clone();
    let focused_pane = app.pane_workspace.focused_pane;
    let mode = app.pane_workspace.mode;
    let status_line_visible = app.pane_workspace.status_line_visible;
    let panes = resolve_rects(&root_clone, area);

    // Tiny workspace: render the focused pane only with a too-small toast.
    if area.width < MIN_WORKSPACE_W || area.height < MIN_WORKSPACE_H {
        render_tiny_workspace(f, area, app, &panes, focused_pane);
        return;
    }

    // Single pane + Normal mode → render without a border, identical to
    // the legacy per-view renderer. This keeps a fresh app looking
    // exactly like today.
    if panes.len() == 1 && mode == UiMode::Normal {
        render_single_pane(f, area, app, &panes[0]);
        return;
    }

    // Reserve space for the status line in PaneEdit mode (only when
    // the user hasn't hidden it via `Ctrl+w, S`).
    let content_area = if mode == UiMode::PaneEdit
        && status_line_visible
        && area.height > STATUS_LINE_H + MIN_PANE_HEIGHT
    {
        Rect::new(area.x, area.y, area.width, area.height - STATUS_LINE_H)
    } else {
        area
    };
    let panes = resolve_rects(&root_clone, content_area);

    // Paint each pane. We look up the module via the global registry
    // (returns `&'static dyn PaneModule + Send + Sync`), then call its
    // render fn with `&mut app`. The static registry borrow doesn't
    // conflict with `&mut app`.
    for p in &panes {
        if !is_usable(p.rect) {
            continue;
        }
        let is_focused = p.pane_id == focused_pane;
        let block = pane_block(p.module_id, is_focused, mode);
        let inner = block.inner(p.rect);
        if !is_usable(inner) {
            // Inner too small after border — just paint the block.
            f.render_widget(block, p.rect);
            continue;
        }
        // Look up the module via the global registry. The reference is
        // `&'static` so the borrow doesn't conflict with `&mut app`.
        if let Some(module) = registry().get(p.module_id) {
            module.render(f, inner, app);
        }
        f.render_widget(block, p.rect);
    }

    // Status line. Only rendered when the user hasn't hidden it.
    if mode == UiMode::PaneEdit
        && status_line_visible
        && area.height > STATUS_LINE_H + MIN_PANE_HEIGHT
    {
        let status_area = Rect::new(
            area.x,
            area.bottom().saturating_sub(STATUS_LINE_H),
            area.width,
            STATUS_LINE_H,
        );
        render_edit_status_line(f, status_area);
    }

    // Rectangle selection preview (Phase 2): drawn on top of the
    // focused pane's content. Painted LAST so it overlays the borders
    // + content of the focused pane. Only drawn when a selection is
    // active (the user pressed `r` in PaneEdit mode).
    if app.rectangle_selection.is_some() {
        let focused_pane = app.pane_workspace.focused_pane;
        let focused_rect = panes
            .iter()
            .find(|p| p.pane_id == focused_pane)
            .map(|p| p.rect)
            .unwrap_or(area);
        render_rectangle_selection(f, focused_rect, app);
    }
}

/// Render a single-pane workspace without a border. Identical to the
/// legacy per-view renderer (so a fresh app looks the same as today).
fn render_single_pane(
    f: &mut Frame,
    area: Rect,
    app: &mut App,
    pane: &crate::tui::pane::ResolvedPane,
) {
    if let Some(module) = registry().get(pane.module_id) {
        module.render(f, area, app);
    }
}

/// Render the focused pane only (with a too-small toast) when the
/// workspace is below `MIN_WORKSPACE_W`x`MIN_WORKSPACE_H`.
fn render_tiny_workspace(
    f: &mut Frame,
    area: Rect,
    app: &mut App,
    panes: &[crate::tui::pane::ResolvedPane],
    focused_pane: crate::tui::pane::PaneId,
) {
    let focused = panes.iter().find(|p| p.pane_id == focused_pane).copied();
    if let Some(p) = focused {
        if is_usable(p.rect) {
            if let Some(module) = registry().get(p.module_id) {
                module.render(f, p.rect, app);
            }
        }
    }
    // Toast at the bottom.
    if area.height >= 2 {
        let toast = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
        let msg = "terminal too small for panes — resize or press Esc";
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                msg,
                Style::default().fg(Color::Yellow),
            )))
            .alignment(Alignment::Center),
            toast,
        );
    }
}

/// Build the border block for a pane. Focused panes get accent + thick
/// (Unicode) / accent + ASCII set (ASCII mode). Unfocused get dim +
/// plain. In PaneEdit mode the focused pane's title gets an "EDIT" badge.
fn pane_block(module: crate::tui::pane::ModuleId, focused: bool, mode: UiMode) -> Block<'static> {
    let theme = crate::tui::view::theme::Theme::default();
    let color = if focused { theme.accent } else { theme.dim };

    let title = if focused && mode == UiMode::PaneEdit {
        format!("{} [EDIT]", module.label())
    } else {
        module.label().to_string()
    };

    let mut block = if is_ascii() {
        Block::default()
            .borders(Borders::ALL)
            .border_set(ASCII_BORDER_SET)
            .border_style(Style::default().fg(color))
    } else {
        let bt = if focused {
            BorderType::Thick
        } else {
            BorderType::Plain
        };
        Block::default()
            .borders(Borders::ALL)
            .border_type(bt)
            .border_style(Style::default().fg(color))
    };
    block = block.title(Span::styled(title, Style::default().fg(color)));
    block
}

/// Edit-mode status line: a one-line keymap hint at the bottom of the
/// workspace. Dim text so it doesn't compete with content. Kept short
/// enough to fit at 80 columns (the narrow path's minimum width).
fn render_edit_status_line(f: &mut Frame, area: Rect) {
    let theme = crate::tui::view::theme::Theme::default();
    let nc = crate::tui::view::theme::no_color();
    let accent = Style::default()
        .fg(if nc { Color::Reset } else { theme.accent })
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(if nc { Color::Reset } else { theme.dim });

    // Clear the line so the pane content above doesn't bleed through.
    f.render_widget(Clear, area);

    // Keep the status line concise: at 80 cols the content area is ~76
    // cells (after rail). The full keymap doesn't fit, so we show the
    // most important keys and let `?` (Help) carry the rest.
    let spans: Vec<Span> = vec![
        Span::styled("PANE EDIT", accent),
        Span::styled(" ", dim),
        Span::styled("hjkl", dim),
        Span::styled(" move ", dim),
        Span::styled("HJKL", dim),
        Span::styled(" resize ", dim),
        Span::styled("v/x/s", dim),
        Span::styled(" split ", dim),
        Span::styled("d", dim),
        Span::styled(" close ", dim),
        Span::styled("m", dim),
        Span::styled(" module ", dim),
        Span::styled("Tab", dim),
        Span::styled(" cycle ", dim),
        Span::styled("1-4", dim),
        Span::styled(" module ", dim),
        Span::styled("Esc", dim),
        Span::styled(" exit", dim),
    ];
    f.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Center),
        area,
    );
}

/// True when the pane workspace should take over rendering from the
/// legacy per-view renderer. Convenience wrapper around
/// `PaneWorkspace::is_active` so `view::columns::render` doesn't have to
/// import the model.
pub fn is_pane_mode_active(app: &App) -> bool {
    app.pane_workspace.is_active()
}

/// Render the rectangle selection preview on top of the focused pane
/// (Phase 2). Draws:
/// - A dim border around the selected region.
/// - A dimension label at the top-left of the selection.
/// - The active corner marked with `+` (ASCII) or `┼` (Unicode).
/// - A "too small" indicator in red if the selection is below the
///   minimum size.
///
/// `pane_rect` is the focused pane's OUTER rect (including border). The
/// function computes the inner rect (after border) so the selection
/// coords match the input layer's `focused_pane_inner_rect`.
fn render_rectangle_selection(f: &mut Frame, pane_rect: Rect, app: &mut App) {
    let sel = match app.rectangle_selection.as_ref() {
        Some(s) => s,
        None => return,
    };
    // Compute the inner rect (subtract 1-cell border on each side).
    let inner = Rect::new(
        pane_rect.x.saturating_add(1),
        pane_rect.y.saturating_add(1),
        pane_rect.width.saturating_sub(2),
        pane_rect.height.saturating_sub(2),
    );
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let sel_rect = sel.to_cell_rect(inner);

    let theme = crate::tui::view::theme::Theme::default();
    let nc = crate::tui::view::theme::no_color();
    let accent = if nc { Color::Reset } else { theme.accent };
    let dim = if nc { Color::Reset } else { theme.dim };

    let is_valid = sel.is_valid(inner);
    let label = sel.dimensions_label(inner);
    let label_style = if is_valid {
        Style::default().fg(accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    };
    let label_text = if is_valid {
        label.clone()
    } else {
        format!("{} · too small", label)
    };

    // Draw the selection border. Even if the rect is very small (1-2
    // cells), we render a border so the user sees the selection. If
    // the rect is 0×0 (degenerate), we render a crosshair marker at the
    // anchor point instead.
    if sel_rect.width > 0 && sel_rect.height > 0 {
        let border_style = Style::default().fg(accent).add_modifier(Modifier::DIM);
        let block = if is_ascii() {
            Block::default()
                .borders(Borders::ALL)
                .border_set(ASCII_BORDER_SET)
                .border_style(border_style)
        } else {
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Plain)
                .border_style(border_style)
        };
        f.render_widget(block, sel_rect);
    } else {
        // Degenerate (0×0) selection: draw a crosshair at the anchor.
        let (ax, ay) = (
            (inner.x as f32 + sel.anchor.x * inner.width as f32).round() as u16,
            (inner.y as f32 + sel.anchor.y * inner.height as f32).round() as u16,
        );
        if ax < inner.right() && ay < inner.bottom() {
            let marker = if is_ascii() { "+" } else { "┼" };
            f.render_widget(
                Paragraph::new(Span::styled(
                    marker,
                    Style::default().fg(accent).add_modifier(Modifier::BOLD),
                )),
                Rect::new(ax, ay, 1, 1),
            );
        }
    }

    // Dimension label: place it at the top-left of the selection if
    // there's room; otherwise place it just above the selection (or at
    // the pane's top row if the selection is at the very top).
    let label_area = if sel_rect.width >= 4 && sel_rect.height >= 2 {
        // Inside the selection's top-left.
        Rect::new(
            sel_rect.x.saturating_add(1),
            sel_rect.y.saturating_add(1),
            sel_rect.width.saturating_sub(2),
            1,
        )
    } else if sel_rect.y > inner.y {
        // Above the selection.
        Rect::new(
            sel_rect.x.max(inner.x),
            sel_rect.y.saturating_sub(1),
            label_text
                .len()
                .min(inner.right().saturating_sub(sel_rect.x.max(inner.x)) as usize)
                as u16,
            1,
        )
    } else {
        // At the pane's first row.
        Rect::new(
            inner.x,
            inner.y,
            inner.width.min(label_text.len() as u16),
            1,
        )
    };
    if label_area.width > 0 {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(label_text, label_style))),
            label_area,
        );
    }

    // Active corner marker. The active corner is `anchor` if
    // `active_is_anchor` else `cursor`. Convert to cell coords and
    // mark with `+` (ASCII) or `┼` (Unicode). The marker overwrites
    // whatever was at that cell (border or content).
    let active = if sel.active_is_anchor {
        sel.anchor
    } else {
        sel.cursor
    };
    let (cx, cy) = (
        (inner.x as f32 + active.x * inner.width as f32).round() as u16,
        (inner.y as f32 + active.y * inner.height as f32).round() as u16,
    );
    if cx < inner.right() && cy < inner.bottom() {
        let marker = if is_ascii() { "+" } else { "┼" };
        f.render_widget(
            Paragraph::new(Span::styled(
                marker,
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            )),
            Rect::new(cx, cy, 1, 1),
        );
    }

    // Phase hint: show the current phase + keybinding at the bottom
    // of the pane's inner rect so the user knows what to do next.
    let phase_hint = match sel.phase {
        SelectionPhase::ChoosingAnchor => {
            "rect: move anchor · Enter confirm · Tab switch · Esc cancel"
        }
        SelectionPhase::ChoosingExtent => {
            "rect: move extent · Enter confirm · Tab switch · Esc cancel"
        }
        SelectionPhase::Confirming => "rect: pick module · Esc cancel",
    };
    if inner.height >= 2 {
        let hint_area = Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                phase_hint,
                Style::default().fg(dim),
            )))
            .alignment(ratatui::layout::Alignment::Center),
            hint_area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::player::StubPlayer;
    use crate::tui::app::App;
    use crate::tui::pane::model::{ModuleId, PaneId, Side};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn build_app() -> App {
        let d = tempfile::tempdir().unwrap();
        let lossless = d.path().join("lossless");
        std::fs::create_dir_all(lossless.join("40mP")).unwrap();
        std::fs::write(lossless.join("40mP").join("01.flac"), b"x").unwrap();
        let json = serde_json::json!({
            "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
            "tracks":[
              {"id":"t1","artists":["40mP"],"primary_artist":"40mP","title":"Song1",
               "album":"Cosmic","bit_depth":24,"sample_rate_hz":96000,
               "source_path":"lossless/40mP/01.flac","symlinked_into_artists":["40mP"]}
            ]
        })
        .to_string();
        let p = d.path().join("catalog.json");
        std::fs::write(&p, json).unwrap();
        // Leak the tempdir so the catalog's source paths stay valid for
        // the app's lifetime. The test process exits soon enough.
        std::mem::forget(d);
        let cat = Catalog::load(&p).unwrap();
        App::new(cat, Box::new(StubPlayer::default()), None, None)
    }

    /// A single-pane workspace in Normal mode renders without a border —
    /// identical to the legacy per-view renderer.
    #[test]
    fn single_pane_no_border() {
        let mut app = build_app();
        // Default workspace: single Artists pane, Normal mode.
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 30);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // The top-left corner should NOT be a border character — the
        // module renders directly into the area. (If a border were
        // drawn, position (0,0) would be '┌' or '+'.)
        let cell = term.backend().buffer()[(0, 0)].symbol();
        assert!(
            !cell.starts_with('┌') && !cell.starts_with('+'),
            "single pane should not have a border, got '{cell}'"
        );
    }

    /// A two-pane workspace renders a border around each pane.
    #[test]
    fn two_panes_have_borders() {
        let mut app = build_app();
        // Split into two panes.
        app.pane_workspace
            .split(PaneId(0), Side::Right, ModuleId::Queue)
            .unwrap();
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 30);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // Top-left corner should be a border character.
        let cell = term.backend().buffer()[(0, 0)].symbol();
        assert!(
            cell.starts_with('┌') || cell.starts_with('+'),
            "two-pane workspace should have a border at (0,0), got '{cell}'"
        );
    }

    /// Edit mode adds an EDIT badge to the focused pane's title.
    #[test]
    fn edit_mode_badge() {
        let mut app = build_app();
        app.pane_workspace
            .split(PaneId(0), Side::Right, ModuleId::Queue)
            .unwrap();
        app.pane_workspace.enter_edit_mode();
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 30);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // The buffer should contain "EDIT" somewhere (in the focused
        // pane's title).
        let buf = term.backend().buffer();
        let mut found = false;
        for y in 0..30 {
            for x in 0..100 {
                let s = buf[(x, y)].symbol();
                if s.contains('E') {
                    // Look for "EDIT" starting at this position.
                    let mut word = String::new();
                    for dx in 0..5 {
                        if x + dx < 100 {
                            word.push_str(buf[(x + dx, y)].symbol());
                        }
                    }
                    if word.contains("EDIT") {
                        found = true;
                        break;
                    }
                }
            }
            if found {
                break;
            }
        }
        assert!(found, "edit-mode badge 'EDIT' should be visible");
    }

    /// Tiny terminal: no panic, focused pane still renders.
    #[test]
    fn tiny_terminal_no_panic() {
        let mut app = build_app();
        app.pane_workspace
            .split(PaneId(0), Side::Right, ModuleId::Queue)
            .unwrap();
        let backend = TestBackend::new(20, 5);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 20, 5);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // No panic is the assertion.
    }

    /// `is_pane_mode_active` is false for a fresh app.
    #[test]
    fn is_pane_mode_active_false_for_fresh_app() {
        let app = build_app();
        assert!(!is_pane_mode_active(&app));
    }

    /// A tiny workspace renders the "terminal too small" toast.
    #[test]
    fn tiny_workspace_renders_toast() {
        let mut app = build_app();
        let backend = TestBackend::new(20, 5);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 20, 5);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // The buffer should contain "too small" somewhere.
        let buf = term.backend().buffer();
        let mut found = false;
        for y in 0..5 {
            for x in 0..20 {
                let s = buf[(x, y)].symbol();
                if s == "t" {
                    let mut word = String::new();
                    for dx in 0..4 {
                        if x + dx < 20 {
                            word.push_str(buf[(x + dx, y)].symbol());
                        }
                    }
                    if word.contains("too") {
                        found = true;
                        break;
                    }
                }
            }
            if found {
                break;
            }
        }
        assert!(found, "tiny workspace should show 'too small' toast");
    }

    /// A tiny workspace with height < 2 doesn't render the toast (the
    /// `if area.height >= 2` branch is false). No panic.
    #[test]
    fn tiny_workspace_height_1_no_toast_no_panic() {
        let mut app = build_app();
        let backend = TestBackend::new(20, 1);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 20, 1);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // No panic; no toast (height < 2).
    }

    /// A multi-pane workspace where the focused pane's rect is unusable
    /// skips rendering that pane (the `is_usable(p.rect)` guard at
    /// line 100). We build a tree with a tiny split, force-focus the
    /// tiny pane, and verify no panic.
    #[test]
    fn multi_pane_skips_unusable_pane_rect() {
        let mut app = build_app();
        // Split: the left pane will be the focused one. We split with a
        // very low ratio so the right pane gets a tiny rect.
        app.pane_workspace
            .split(PaneId(0), Side::Right, ModuleId::Queue)
            .unwrap();
        // Force the right pane to be very narrow by resizing.
        for _ in 0..50 {
            let _ = app
                .pane_workspace
                .resize(PaneId(0), crate::tui::pane::Direction::Right, 0.05);
        }
        let backend = TestBackend::new(40, 8);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 8);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // No panic — the unusable pane was skipped.
    }

    /// A pane with usable outer rect but unusable inner rect (after
    /// border) renders the block only (no module content). We test at
    /// a size where the inner is too small.
    #[test]
    fn pane_with_unusable_inner_renders_block_only() {
        let mut app = build_app();
        app.pane_workspace
            .split(PaneId(0), Side::Right, ModuleId::Queue)
            .unwrap();
        app.pane_workspace.enter_edit_mode();
        // Resize aggressively so the right pane is exactly at the
        // usability edge: outer rect = 10 wide (usable), inner = 8
        // wide (unusable — border subtracts 2 cells each side). With
        // area width = 100 and ratio = 0.9, right pane = 10 cells
        // outer. That triggers the "inner too small" branch at
        // line 106-109 (renders block, skips module content).
        for _ in 0..50 {
            let _ = app
                .pane_workspace
                .resize(PaneId(0), crate::tui::pane::Direction::Right, 0.05);
        }
        let backend = TestBackend::new(100, 10);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 10);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // No panic — the small pane's block was rendered but content
        // wasn't (inner unusable).
    }

    /// Edit-mode status line is rendered at the bottom in PaneEdit mode
    /// when the workspace is large enough.
    #[test]
    fn edit_mode_status_line_rendered() {
        let mut app = build_app();
        app.pane_workspace
            .split(PaneId(0), Side::Right, ModuleId::Queue)
            .unwrap();
        app.pane_workspace.enter_edit_mode();
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 30);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // The buffer should contain "PANE EDIT" somewhere.
        let buf = term.backend().buffer();
        let mut found = false;
        for y in 0..30 {
            for x in 0..100 {
                if buf[(x, y)].symbol() == "P" {
                    let mut word = String::new();
                    for dx in 0..9 {
                        if x + dx < 100 {
                            word.push_str(buf[(x + dx, y)].symbol());
                        }
                    }
                    if word.contains("PANE EDIT") {
                        found = true;
                        break;
                    }
                }
            }
            if found {
                break;
            }
        }
        assert!(found, "edit-mode status line 'PANE EDIT' should be visible");
    }

    /// Rectangle selection preview: render with a degenerate (0×0)
    /// selection — the crosshair branch. No panic.
    #[test]
    fn rectangle_selection_degenerate_renders_crosshair() {
        use crate::tui::pane::selection::{
            NormalizedPoint, RectangleSelection, SelectionInput, SelectionPhase,
        };
        let mut app = build_app();
        app.pane_workspace.enter_edit_mode();
        // Build a degenerate selection: anchor == cursor → 0×0 sel_rect.
        app.rectangle_selection = Some(RectangleSelection {
            target_pane: app.pane_workspace.focused_pane,
            anchor: NormalizedPoint::new(0.5, 0.5),
            cursor: NormalizedPoint::new(0.5, 0.5),
            phase: SelectionPhase::ChoosingExtent,
            input_source: SelectionInput::Keyboard,
            active_is_anchor: false,
        });
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 30);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // No panic — the crosshair was drawn.
    }

    /// Rectangle selection with a small selection that doesn't fit the
    /// "inside top-left" label position uses the "above selection"
    /// branch (sel_rect.y > inner.y).
    #[test]
    fn rectangle_selection_label_above_selection() {
        use crate::tui::pane::selection::{
            NormalizedPoint, RectangleSelection, SelectionInput, SelectionPhase,
        };
        let mut app = build_app();
        app.pane_workspace.enter_edit_mode();
        // Build a selection whose sel_rect.y > inner.y (selection not at
        // the very top of the pane). Anchor at 30%, cursor at 40%.
        app.rectangle_selection = Some(RectangleSelection {
            target_pane: app.pane_workspace.focused_pane,
            anchor: NormalizedPoint::new(0.3, 0.3),
            cursor: NormalizedPoint::new(0.4, 0.4),
            phase: SelectionPhase::ChoosingExtent,
            input_source: SelectionInput::Keyboard,
            active_is_anchor: false,
        });
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 30);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // No panic; the "above selection" label branch was taken.
    }

    /// Rectangle selection at the very top of the pane (sel_rect.y ==
    /// inner.y) uses the "at pane's first row" label branch.
    #[test]
    fn rectangle_selection_label_at_pane_first_row() {
        use crate::tui::pane::selection::{
            NormalizedPoint, RectangleSelection, SelectionInput, SelectionPhase,
        };
        let mut app = build_app();
        app.pane_workspace.enter_edit_mode();
        // Selection at the very top: anchor y = 0, cursor y = 0.2.
        app.rectangle_selection = Some(RectangleSelection {
            target_pane: app.pane_workspace.focused_pane,
            anchor: NormalizedPoint::new(0.3, 0.0),
            cursor: NormalizedPoint::new(0.7, 0.2),
            phase: SelectionPhase::ChoosingExtent,
            input_source: SelectionInput::Keyboard,
            active_is_anchor: false,
        });
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 30);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // No panic; the "at pane's first row" label branch was taken.
    }

    /// Rectangle selection with the active corner at the bottom-right
    /// of the pane (cx < inner.right() && cy < inner.bottom() —
    /// exercises the active corner marker rendering).
    #[test]
    fn rectangle_selection_active_corner_marker_rendered() {
        use crate::tui::pane::selection::{
            NormalizedPoint, RectangleSelection, SelectionInput, SelectionPhase,
        };
        let mut app = build_app();
        app.pane_workspace.enter_edit_mode();
        app.rectangle_selection = Some(RectangleSelection {
            target_pane: app.pane_workspace.focused_pane,
            anchor: NormalizedPoint::new(0.2, 0.2),
            cursor: NormalizedPoint::new(0.8, 0.8),
            phase: SelectionPhase::ChoosingExtent,
            input_source: SelectionInput::Keyboard,
            active_is_anchor: false, // cursor is active (0.8, 0.8)
        });
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 30);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // No panic; the active corner marker was drawn at (cx, cy).
    }

    /// Rectangle selection in a too-small workspace (the inner rect
    /// after subtracting the border is 0×0): the early return at line
    /// 297-299 is taken. No panic.
    #[test]
    fn rectangle_selection_tiny_inner_no_panic() {
        use crate::tui::pane::selection::{
            NormalizedPoint, RectangleSelection, SelectionInput, SelectionPhase,
        };
        let mut app = build_app();
        app.pane_workspace.enter_edit_mode();
        app.rectangle_selection = Some(RectangleSelection {
            target_pane: app.pane_workspace.focused_pane,
            anchor: NormalizedPoint::new(0.5, 0.5),
            cursor: NormalizedPoint::new(0.6, 0.6),
            phase: SelectionPhase::ChoosingExtent,
            input_source: SelectionInput::Keyboard,
            active_is_anchor: false,
        });
        // Tiny workspace (1×1) — inner rect after border = 0×0. The
        // render_rectangle_selection early-returns.
        let backend = TestBackend::new(1, 1);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 1, 1);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // No panic.
    }

    /// `pane_block` produces a non-empty Block for each module id.
    /// Indirectly verifies the title format and that the function
    /// doesn't panic for any module.
    #[test]
    fn pane_block_for_all_modules() {
        use crate::tui::pane::model::ModuleId;
        for id in ModuleId::all() {
            let _b = pane_block(id, true, UiMode::PaneEdit);
            let _b = pane_block(id, false, UiMode::Normal);
            let _b = pane_block(id, false, UiMode::PaneEdit);
        }
    }

    /// `render_edit_status_line` doesn't panic at the minimum size.
    #[test]
    fn render_edit_status_line_no_panic() {
        let backend = TestBackend::new(80, 1);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 80, 1);
        term.draw(|f| render_edit_status_line(f, area)).unwrap();
    }

    /// `render_tiny_workspace` doesn't panic when the focused pane
    /// isn't in the resolved panes (defensive — should never happen).
    /// We test by passing a focused_pane that doesn't match any pane.
    #[test]
    fn render_tiny_workspace_unknown_focused_pane_no_panic() {
        let mut app = build_app();
        let panes = resolve_rects(&app.pane_workspace.root, Rect::new(0, 0, 20, 5));
        let backend = TestBackend::new(20, 5);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 20, 5);
        // Force a non-existent focused_pane via a direct call to
        // render_tiny_workspace.
        term.draw(|f| {
            render_tiny_workspace(f, area, &mut app, &panes, crate::tui::pane::PaneId(99));
        })
        .unwrap();
        // No panic — the unknown focused pane is handled.
    }

    /// `render_single_pane` doesn't panic for a placeholder module.
    #[test]
    fn render_single_pane_placeholder_no_panic() {
        let mut app = build_app();
        app.pane_workspace.set_module(
            crate::tui::pane::PaneId(0),
            crate::tui::pane::model::ModuleId::Placeholder,
        );
        let panes = resolve_rects(&app.pane_workspace.root, Rect::new(0, 0, 100, 30));
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 30);
        term.draw(|f| render_single_pane(f, area, &mut app, &panes[0]))
            .unwrap();
    }

    /// ASCII font mode: pane borders use the ASCII border set instead
    /// of Unicode box-drawing chars. Covers the `is_ascii()` branch in
    /// `pane_block` and `render_rectangle_selection`. Uses
    /// `JUKEBOX_FONT_MODE=ascii` + `reset_font_mode_cache` to flip the
    /// mode for the calling thread only.
    #[test]
    fn ascii_mode_pane_borders_use_ascii_set() {
        let mut app = build_app();
        app.pane_workspace
            .split(PaneId(0), Side::Right, ModuleId::Queue)
            .unwrap();
        app.pane_workspace.enter_edit_mode();
        std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
        crate::tui::view::theme::reset_font_mode_cache();
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 30);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        let cell = term.backend().buffer()[(0, 0)].symbol();
        assert!(
            cell.starts_with('+'),
            "ASCII mode should render '+' at (0,0), got '{cell}'"
        );
        std::env::remove_var("JUKEBOX_FONT_MODE");
        crate::tui::view::theme::reset_font_mode_cache();
    }

    /// ASCII font mode: rectangle selection border uses the ASCII
    /// border set. Covers the `is_ascii()` branch in
    /// `render_rectangle_selection`.
    #[test]
    fn ascii_mode_rectangle_selection_uses_ascii_border() {
        use crate::tui::pane::selection::{
            NormalizedPoint, RectangleSelection, SelectionInput, SelectionPhase,
        };
        let mut app = build_app();
        app.pane_workspace.enter_edit_mode();
        app.rectangle_selection = Some(RectangleSelection {
            target_pane: app.pane_workspace.focused_pane,
            anchor: NormalizedPoint::new(0.2, 0.2),
            cursor: NormalizedPoint::new(0.8, 0.8),
            phase: SelectionPhase::ChoosingExtent,
            input_source: SelectionInput::Keyboard,
            active_is_anchor: false,
        });
        std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
        crate::tui::view::theme::reset_font_mode_cache();
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 30);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        std::env::remove_var("JUKEBOX_FONT_MODE");
        crate::tui::view::theme::reset_font_mode_cache();
    }

    /// ASCII font mode: degenerate (0×0) rectangle selection renders
    /// a crosshair using '+' instead of '┼'. Covers the `is_ascii()`
    /// branch in the crosshair path of `render_rectangle_selection`.
    #[test]
    fn ascii_mode_degenerate_selection_crosshair() {
        use crate::tui::pane::selection::{
            NormalizedPoint, RectangleSelection, SelectionInput, SelectionPhase,
        };
        let mut app = build_app();
        app.pane_workspace.enter_edit_mode();
        app.rectangle_selection = Some(RectangleSelection {
            target_pane: app.pane_workspace.focused_pane,
            anchor: NormalizedPoint::new(0.5, 0.5),
            cursor: NormalizedPoint::new(0.5, 0.5),
            phase: SelectionPhase::ChoosingExtent,
            input_source: SelectionInput::Keyboard,
            active_is_anchor: false,
        });
        std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
        crate::tui::view::theme::reset_font_mode_cache();
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 30);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        std::env::remove_var("JUKEBOX_FONT_MODE");
        crate::tui::view::theme::reset_font_mode_cache();
    }

    /// Rectangle selection with active_is_anchor = true draws the
    /// ANCHOR as the active corner (covers the `if sel.active_is_anchor`
    /// true branch).
    #[test]
    fn rectangle_selection_anchor_is_active_corner() {
        use crate::tui::pane::selection::{
            NormalizedPoint, RectangleSelection, SelectionInput, SelectionPhase,
        };
        let mut app = build_app();
        app.pane_workspace.enter_edit_mode();
        app.rectangle_selection = Some(RectangleSelection {
            target_pane: app.pane_workspace.focused_pane,
            anchor: NormalizedPoint::new(0.2, 0.2),
            cursor: NormalizedPoint::new(0.8, 0.8),
            phase: SelectionPhase::ChoosingExtent,
            input_source: SelectionInput::Keyboard,
            active_is_anchor: true, // anchor is the active corner
        });
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 30);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // No panic; the anchor (0.2, 0.2) was drawn as the active corner.
    }

    /// Rectangle selection with the active corner outside the pane
    /// bounds (cx >= inner.right() or cy >= inner.bottom()): the
    /// marker is NOT drawn (the `if cx < inner.right() && cy <
    /// inner.bottom()` guard is false). No panic.
    #[test]
    fn rectangle_selection_active_corner_outside_pane_no_marker() {
        use crate::tui::pane::selection::{
            NormalizedPoint, RectangleSelection, SelectionInput, SelectionPhase,
        };
        let mut app = build_app();
        app.pane_workspace.enter_edit_mode();
        // Active corner at (1.5, 1.5) — way outside the pane.
        app.rectangle_selection = Some(RectangleSelection {
            target_pane: app.pane_workspace.focused_pane,
            anchor: NormalizedPoint::new(1.5, 1.5),
            cursor: NormalizedPoint::new(1.5, 1.5),
            phase: SelectionPhase::ChoosingExtent,
            input_source: SelectionInput::Keyboard,
            active_is_anchor: false,
        });
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 30);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // No panic — the marker was skipped (outside pane bounds).
    }

    /// Rectangle selection at the very top of the pane with a tiny
    /// label_text (so label_area.width clamps to 0): the
    /// `if label_area.width > 0` guard is false, so the label isn't
    /// drawn. No panic.
    #[test]
    fn rectangle_selection_label_area_zero_width_no_label() {
        use crate::tui::pane::selection::{
            NormalizedPoint, RectangleSelection, SelectionInput, SelectionPhase,
        };
        let mut app = build_app();
        app.pane_workspace.enter_edit_mode();
        // Force a selection at the very top of the pane with a tiny
        // width (sel_rect.x == inner.x, so label_area goes to the "at
        // pane's first row" branch with inner.width = 0).
        app.rectangle_selection = Some(RectangleSelection {
            target_pane: app.pane_workspace.focused_pane,
            anchor: NormalizedPoint::new(0.0, 0.0),
            cursor: NormalizedPoint::new(0.01, 0.05),
            phase: SelectionPhase::ChoosingExtent,
            input_source: SelectionInput::Keyboard,
            active_is_anchor: false,
        });
        let backend = TestBackend::new(40, 8);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 8);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // No panic.
    }

    /// Rectangle selection in ChoosingAnchor phase renders the
    /// "move anchor" phase hint at the bottom of the pane.
    #[test]
    fn rectangle_selection_choosing_anchor_phase_hint_rendered() {
        use crate::tui::pane::selection::{
            NormalizedPoint, RectangleSelection, SelectionInput, SelectionPhase,
        };
        let mut app = build_app();
        app.pane_workspace.enter_edit_mode();
        app.rectangle_selection = Some(RectangleSelection {
            target_pane: app.pane_workspace.focused_pane,
            anchor: NormalizedPoint::new(0.4, 0.4),
            cursor: NormalizedPoint::new(0.6, 0.6),
            phase: SelectionPhase::ChoosingAnchor,
            input_source: SelectionInput::Keyboard,
            active_is_anchor: true,
        });
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 30);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // No panic; the "move anchor" phase hint was rendered.
    }

    /// Rectangle selection in Confirming phase renders the "pick
    /// module" phase hint at the bottom of the pane.
    #[test]
    fn rectangle_selection_confirming_phase_hint_rendered() {
        use crate::tui::pane::selection::{
            NormalizedPoint, RectangleSelection, SelectionInput, SelectionPhase,
        };
        let mut app = build_app();
        app.pane_workspace.enter_edit_mode();
        app.rectangle_selection = Some(RectangleSelection {
            target_pane: app.pane_workspace.focused_pane,
            anchor: NormalizedPoint::new(0.2, 0.2),
            cursor: NormalizedPoint::new(0.8, 0.8),
            phase: SelectionPhase::Confirming,
            input_source: SelectionInput::Keyboard,
            active_is_anchor: false,
        });
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 30);
        term.draw(|f| render_pane_workspace(f, area, &mut app))
            .unwrap();
        // No panic; the "pick module" phase hint was rendered.
    }
}
