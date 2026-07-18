//! Pane data model + pure mutation.
//!
//! The split tree stores normalized ratios (never terminal cells). Layout
//! is recomputed from the tree on every render via [`crate::tui::pane::layout`].
//! All mutation is pure with respect to the terminal — none of these
//! functions read from or write to the TTY.

use serde::{Deserialize, Serialize};

/// Minimum ratio of the first child in a split. Below this the child
/// becomes unusably small; [`clamp_ratio`] enforces it on every mutation.
pub const MIN_RATIO: f32 = 0.1;
/// Maximum ratio of the first child (so the second child is at least
/// `1.0 - MAX_RATIO`). Symmetric with [`MIN_RATIO`].
pub const MAX_RATIO: f32 = 0.9;
/// Default ratio for a fresh split (50/50).
pub const DEFAULT_RATIO: f32 = 0.5;
/// Step used by resize keybindings (2% per press).
pub const RESIZE_STEP: f32 = 0.02;

/// Stable pane identifier. Monotonic within a session — never reused. The
/// layout engine identifies panes by `PaneId`, never by their current
/// `Rect`, so terminal resize never invalidates focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PaneId(pub u64);

/// Which registered module a leaf pane is rendering. Extensible — new
/// variants are added here; the registry dispatches on this.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ModuleId {
    #[default]
    Artists,
    Playlists,
    Queue,
    Youtube,
    /// The big "Now Playing" player bar as a pane module. Renders the
    /// full-size now-playing view (title, artist, album, quality, big
    /// progress bar, transport, source badge, next-up preview) into a
    /// pane. Useful for dedicating a pane to playback status. Wraps
    /// `view::player_bar_big::render_big` + `view::now_playing_panel`.
    NowPlaying,
    /// YouTube Home sub-tab as a pane module. Renders just the Home
    /// content (Quick Picks, mixes, radio shelves) without the sub-tab
    /// bar. Wraps `view::yt_view::render_yt_home`.
    YtHome,
    /// YouTube Library sub-tab (account + suggested + generated playlists).
    YtLibrary,
    /// YouTube Search sub-tab (search input + results).
    YtSearch,
    /// YouTube Discover sub-tab (suggested albums / YT mood playlists).
    YtDiscover,
    /// YouTube Radio sub-tab (radio session UI).
    YtRadio,
    /// YouTube Explore sub-tab (explore-feed playlists).
    YtExplore,
    /// YouTube Charts sub-tab (chart entries grouped by chart type).
    YtCharts,
    /// Demo / placeholder module proving third-party modules can register.
    /// Rendered as a "Press `m` to choose a module" hint.
    Placeholder,
}

impl ModuleId {
    /// All registered built-in modules, in the order they appear in the
    /// module picker. `Placeholder` is last (it's the "no choice" default
    /// for a fresh split) so the user sees real modules first.
    pub fn all() -> [ModuleId; 13] {
        [
            ModuleId::Artists,
            ModuleId::Playlists,
            ModuleId::Queue,
            ModuleId::Youtube,
            ModuleId::NowPlaying,
            ModuleId::YtHome,
            ModuleId::YtLibrary,
            ModuleId::YtSearch,
            ModuleId::YtDiscover,
            ModuleId::YtRadio,
            ModuleId::YtExplore,
            ModuleId::YtCharts,
            ModuleId::Placeholder,
        ]
    }

    /// Human-readable label for the pane title + module picker.
    pub fn label(self) -> &'static str {
        match self {
            ModuleId::Artists => "Artists",
            ModuleId::Playlists => "Playlists",
            ModuleId::Queue => "Queue",
            ModuleId::Youtube => "YouTube",
            ModuleId::NowPlaying => "Now Playing",
            ModuleId::YtHome => "YT Home",
            ModuleId::YtLibrary => "YT Library",
            ModuleId::YtSearch => "YT Search",
            ModuleId::YtDiscover => "YT Discover",
            ModuleId::YtRadio => "YT Radio",
            ModuleId::YtExplore => "YT Explore",
            ModuleId::YtCharts => "YT Charts",
            ModuleId::Placeholder => "Placeholder",
        }
    }
}

/// Which axis a split divides its children along.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SplitAxis {
    /// Children stacked top/bottom (a horizontal dividing line).
    Horizontal,
    /// Children side-by-side left/right (a vertical dividing line).
    Vertical,
}

/// A node in the split tree. Either a leaf (a single module) or a split
/// (two children divided along an axis at a normalized ratio).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PaneNode {
    Leaf {
        id: PaneId,
        module: ModuleId,
    },
    Split {
        axis: SplitAxis,
        /// Normalized share of the FIRST child, in `[MIN_RATIO, MAX_RATIO]`.
        /// `1.0 - ratio` is the second child's share. Stored as a normalized
        /// float so terminal resize requires no layout mutation.
        ratio: f32,
        first: Box<PaneNode>,
        second: Box<PaneNode>,
    },
}

impl PaneNode {
    /// True if this is a leaf with the given id.
    pub fn is_leaf_with_id(&self, target: PaneId) -> bool {
        matches!(self, PaneNode::Leaf { id, .. } if *id == target)
    }

    /// Number of leaves under this node.
    pub fn leaf_count(&self) -> usize {
        match self {
            PaneNode::Leaf { .. } => 1,
            PaneNode::Split { first, second, .. } => first.leaf_count() + second.leaf_count(),
        }
    }
}

/// Interaction mode. Stored on `PaneWorkspace` — separate from the
/// module-internal focus state (e.g. which artist row is highlighted).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum UiMode {
    /// Input goes to the focused pane's module; global keybindings work.
    /// Pane focus can be moved via the `Ctrl+w` prefix.
    #[default]
    Normal,
    /// Pane-edit keybindings active. Module-local navigation is suppressed.
    /// Global playback + quit keys still work.
    PaneEdit,
    /// Module-picker overlay open. The overlay takes input.
    PaneModulePicker,
}

/// Which side of the focused pane to split. Determines the new pane's
/// position and the parent split's axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    /// The axis of the split created by this side. Left/Right → Vertical
    /// (a vertical line divides left from right). Top/Bottom → Horizontal
    /// (a horizontal line divides top from bottom).
    pub fn axis(self) -> SplitAxis {
        match self {
            Side::Left | Side::Right => SplitAxis::Vertical,
            Side::Top | Side::Bottom => SplitAxis::Horizontal,
        }
    }

    /// Whether the new pane becomes the first (Left/Top) or second
    /// (Right/Bottom) child of the new split. Per the spec: splitting
    /// left/top inserts the new pane as the first child.
    pub fn new_is_first(self) -> bool {
        matches!(self, Side::Left | Side::Top)
    }
}

/// Spatial direction for focus movement + resize.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Which child of a split a path step descends into. Used by tree
/// traversal helpers (`find_path`, `resize_recursive`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildPos {
    First,
    Second,
}

/// Clamp a ratio to `[MIN_RATIO, MAX_RATIO]`.
pub fn clamp_ratio(r: f32) -> f32 {
    r.clamp(MIN_RATIO, MAX_RATIO)
}

#[derive(Debug, PartialEq, Eq)]
pub enum SplitError {
    /// The target pane id was not found in the tree.
    NotFound,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CloseError {
    /// The target pane id was not found in the tree.
    NotFound,
    /// The target is the root leaf — refusing to close the last pane.
    IsRoot,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ResizeError {
    /// The target pane id was not found in the tree.
    NotFound,
    /// No ancestor split can affect the requested boundary (e.g. the
    /// focused pane already spans the full width/height of the workspace
    /// in the requested axis).
    NoAncestor,
}

/// The pane workspace. Owned by `App` as `app.pane_workspace`. All layout
/// mutation goes through methods on this struct; the render layer reads
/// `root` + `focused_pane` + `mode` and never mutates them.
pub struct PaneWorkspace {
    /// Root of the split tree. Always non-empty.
    pub root: PaneNode,
    /// The currently focused leaf pane. Always a valid leaf id in `root`.
    pub focused_pane: PaneId,
    /// Current interaction mode.
    pub mode: UiMode,
    /// Monotonic id generator. Bumped on every `split` so ids stay unique
    /// across the session. Public so the persistence DTO can read/write
    /// it (the DTO must restore the counter to avoid id reuse after a
    /// restart).
    pub next_id: u64,
    /// Whether the PANE EDIT status line (the one-line keymap hint at the
    /// bottom of the workspace) is visible. Toggled by `Ctrl+w, S` or `S`
    /// in PaneEdit mode. Persisted across sessions via
    /// [`crate::tui::pane::persistence::PaneWorkspaceDto`]. Default true.
    pub status_line_visible: bool,
}

impl PaneWorkspace {
    /// Build a default workspace: a single root leaf (Artists module),
    /// focused, in Normal mode. This is what a fresh launch looks like —
    /// the pane system is invisible until the user splits or enters edit
    /// mode.
    pub fn new() -> Self {
        let root = PaneNode::Leaf {
            id: PaneId(0),
            module: ModuleId::Artists,
        };
        Self {
            root,
            focused_pane: PaneId(0),
            mode: UiMode::Normal,
            next_id: 1,
            status_line_visible: true,
        }
    }

    /// True when the pane workspace should take over rendering from the
    /// legacy per-view renderer. The workspace is "active" when there's
    /// more than one pane OR when in PaneEdit mode. When inactive, the
    /// existing `view::columns::render_*` functions render directly into
    /// the content area (so a fresh app looks identical to today).
    pub fn is_active(&self) -> bool {
        match self.mode {
            UiMode::PaneEdit | UiMode::PaneModulePicker => true,
            UiMode::Normal => self.root.leaf_count() > 1,
        }
    }

    /// Allocate the next pane id. Bumps `next_id` so the next call returns
    /// a fresh value. Used by `split`.
    pub fn next_pane_id(&mut self) -> PaneId {
        let id = PaneId(self.next_id);
        self.next_id += 1;
        id
    }

    pub fn enter_edit_mode(&mut self) {
        self.mode = UiMode::PaneEdit;
    }

    pub fn exit_edit_mode(&mut self) {
        self.mode = UiMode::Normal;
    }

    pub fn set_mode(&mut self, mode: UiMode) {
        self.mode = mode;
    }

    /// Toggle the PANE EDIT status line (the one-line keymap hint at the
    /// bottom of the workspace). Bound to `Ctrl+w, S` (any mode) and `S`
    /// (PaneEdit mode only, since `s` lowercase is the split picker).
    /// Persisted across sessions via the DTO.
    pub fn toggle_status_line(&mut self) {
        self.status_line_visible = !self.status_line_visible;
    }
    /// True if `pane` is a leaf id in the tree.
    pub fn contains(&self, pane: PaneId) -> bool {
        find_path(&self.root, pane).is_some()
    }

    /// Set the focused pane. No-op if `pane` isn't a leaf in the tree
    /// (defensive — never panics).
    pub fn set_focused(&mut self, pane: PaneId) {
        if self.contains(pane) {
            self.focused_pane = pane;
        }
    }

    /// Split the target pane on the given side, installing `module` in
    /// the new pane. The new pane is focused.
    ///
    /// Split semantics (per the spec):
    /// - Left/Top → new pane is the FIRST child of the new split.
    /// - Right/Bottom → new pane is the SECOND child.
    /// - The new split's ratio is `DEFAULT_RATIO` (50/50), clamped.
    /// - The new pane gets a fresh `PaneId` (so ids stay unique across
    ///   repeated splits).
    pub fn split(
        &mut self,
        target: PaneId,
        side: Side,
        module: ModuleId,
    ) -> Result<PaneId, SplitError> {
        let new_id = self.next_pane_id();
        if !split_leaf(&mut self.root, target, side, new_id, module) {
            // Put the id back so we don't burn through ids on a failed split
            // (the failure is a no-op; the next successful split reuses it).
            self.next_id -= 1;
            return Err(SplitError::NotFound);
        }
        self.focused_pane = new_id;
        Ok(new_id)
    }

    /// Close the target pane. Its parent split is replaced in-place by
    /// the surviving sibling. Focus moves to the first leaf of the
    /// surviving subtree. Refuses if `target` is the root leaf.
    pub fn close(&mut self, target: PaneId) -> Result<(), CloseError> {
        let mut new_focus: Option<PaneId> = None;
        match close_leaf(&mut self.root, target, &mut new_focus) {
            CloseResult::Closed => {
                self.focused_pane = new_focus.unwrap_or_else(|| first_leaf_id(&self.root));
                Ok(())
            }
            CloseResult::NotFound => Err(CloseError::NotFound),
            CloseResult::IsRoot => Err(CloseError::IsRoot),
        }
    }

    /// Grow the focused pane in `dir` by `step` (a normalized ratio delta,
    /// e.g. `RESIZE_STEP`). Adjusts the ratio of the nearest ancestor
    /// split whose axis can affect the requested boundary. No-op (returns
    /// `NoAncestor`) if no such ancestor exists.
    pub fn resize(&mut self, target: PaneId, dir: Direction, step: f32) -> Result<(), ResizeError> {
        let path = find_path(&self.root, target).ok_or(ResizeError::NotFound)?;
        match resize_recursive(&mut self.root, &path, dir, step) {
            ResizeResult::Resized => Ok(()),
            ResizeResult::NoAncestor => Err(ResizeError::NoAncestor),
        }
    }

    /// Change the module assigned to a leaf pane. No-op if the target
    /// isn't found. Returns true if the module was changed.
    pub fn set_module(&mut self, target: PaneId, module: ModuleId) -> bool {
        set_module_leaf(&mut self.root, target, module)
    }

    /// Move focus in a direction using the resolved pane rects. Returns
    /// true if focus moved. Delegates to `focus::move_focus_directional`.
    pub fn move_focus(&mut self, panes: &[super::layout::ResolvedPane], dir: Direction) -> bool {
        if let Some(new) = super::focus::move_focus_directional(panes, self.focused_pane, dir) {
            self.focused_pane = new;
            true
        } else {
            false
        }
    }

    /// Cycle focus to the next/previous leaf in tree order. Fallback when
    /// directional movement has no candidate.
    pub fn cycle_focus(&mut self, panes: &[super::layout::ResolvedPane], forward: bool) -> bool {
        if let Some(new) = super::focus::cycle_focus(panes, self.focused_pane, forward) {
            self.focused_pane = new;
            true
        } else {
            false
        }
    }

    /// Replace the target leaf with the subtree built from a rectangle
    /// selection (see [`super::selection::RectangleSelection::convert_to_splits`]).
    /// The chosen `module` is installed in the center pane; surrounding
    /// panes get [`ModuleId::Placeholder`]. All new panes get fresh ids
    /// (allocated from `next_id`); the center pane becomes focused.
    ///
    /// Returns `true` if the target was found and replaced. Returns
    /// `false` if the target wasn't found (no mutation, no id burn —
    /// defensive: the caller passes a stale id).
    pub fn apply_rectangle_selection(
        &mut self,
        selection: &super::selection::RectangleSelection,
        module: ModuleId,
    ) -> bool {
        // Build the subtree with placeholder ids (PaneId(0)..PaneId(n)).
        let (subtree, _new_ids) = selection.convert_to_splits(module);
        // Reassign ids to fresh ones from the workspace's counter. The
        // center pane is the leaf that was PaneId(0) in the placeholder
        // space; we track it through reassignment so we can focus it.
        let mut counter = self.next_id;
        let mut center_id: Option<PaneId> = None;
        let mut new_subtree = subtree;
        reassign_ids(&mut new_subtree, &mut counter, &mut center_id);
        // Replace the target leaf with the new subtree.
        if !replace_leaf(&mut self.root, selection.target_pane, new_subtree) {
            // Target not found — don't burn ids. Caller error; no
            // mutation performed.
            return false;
        }
        self.next_id = counter;
        if let Some(cid) = center_id {
            self.focused_pane = cid;
        }
        true
    }
}

/// Walk the subtree and reassign each leaf id to a fresh id from
/// `counter`. The leaf that was `PaneId(0)` (the center, created first
/// by `convert_to_splits`) is recorded in `center_id` so the caller can
/// focus it.
fn reassign_ids(node: &mut PaneNode, counter: &mut u64, center_id: &mut Option<PaneId>) {
    match node {
        PaneNode::Leaf { id, .. } => {
            let was_center = *id == PaneId(0);
            *id = PaneId(*counter);
            *counter += 1;
            if was_center {
                *center_id = Some(*id);
            }
        }
        PaneNode::Split { first, second, .. } => {
            reassign_ids(first, counter, center_id);
            reassign_ids(second, counter, center_id);
        }
    }
}

/// Replace the target leaf in `node` with `replacement`. Returns true if
/// the target was found and replaced. Recurses into split children.
/// Clones `replacement` for the first-child attempt so it can be moved
/// into the second child if the first doesn't contain the target.
fn replace_leaf(node: &mut PaneNode, target: PaneId, replacement: PaneNode) -> bool {
    if node.is_leaf_with_id(target) {
        *node = replacement;
        return true;
    }
    if let PaneNode::Split { first, second, .. } = node {
        if replace_leaf(first, target, replacement.clone()) {
            return true;
        }
        if replace_leaf(second, target, replacement) {
            return true;
        }
    }
    false
}

impl Default for PaneWorkspace {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Pure tree mutation helpers
// ---------------------------------------------------------------------------

/// Find the path from `node` to `target`. Returns `Some(path)` where
/// `path[0]` is which child of `node` to descend into, `path[len-1]` is
/// which child of `target`'s parent. `path.len() == 0` means `target` is
/// `node` itself (the root). `None` if `target` isn't in the subtree.
pub fn find_path(node: &PaneNode, target: PaneId) -> Option<Vec<ChildPos>> {
    match node {
        PaneNode::Leaf { id, .. } => {
            if *id == target {
                Some(vec![])
            } else {
                None
            }
        }
        PaneNode::Split { first, second, .. } => {
            if let Some(mut p) = find_path(first, target) {
                p.insert(0, ChildPos::First);
                Some(p)
            } else if let Some(mut p) = find_path(second, target) {
                p.insert(0, ChildPos::Second);
                Some(p)
            } else {
                None
            }
        }
    }
}

/// The id of the first (leftmost/topmost) leaf under `node`.
pub fn first_leaf_id(node: &PaneNode) -> PaneId {
    match node {
        PaneNode::Leaf { id, .. } => *id,
        PaneNode::Split { first, .. } => first_leaf_id(first),
    }
}

/// All leaf ids under `node`, in left-to-right / top-to-bottom order.
pub fn leaf_ids(node: &PaneNode) -> Vec<PaneId> {
    let mut out = Vec::new();
    collect_leaf_ids(node, &mut out);
    out
}

fn collect_leaf_ids(node: &PaneNode, out: &mut Vec<PaneId>) {
    match node {
        PaneNode::Leaf { id, .. } => out.push(*id),
        PaneNode::Split { first, second, .. } => {
            collect_leaf_ids(first, out);
            collect_leaf_ids(second, out);
        }
    }
}

/// Split the target leaf in-place, replacing it with a new Split node
/// whose children are the old leaf and a new leaf. Returns true if the
/// target was found and split; false otherwise.
fn split_leaf(
    node: &mut PaneNode,
    target: PaneId,
    side: Side,
    new_id: PaneId,
    new_module: ModuleId,
) -> bool {
    // If THIS is the target leaf, replace it with a Split.
    if node.is_leaf_with_id(target) {
        // Take the old leaf out so we can move it into the new Split.
        let old_leaf = std::mem::replace(
            node,
            PaneNode::Leaf {
                id: PaneId(u64::MAX),
                module: ModuleId::Placeholder,
            },
        );
        let new_leaf = PaneNode::Leaf {
            id: new_id,
            module: new_module,
        };
        let (first, second) = if side.new_is_first() {
            (new_leaf, old_leaf)
        } else {
            (old_leaf, new_leaf)
        };
        *node = PaneNode::Split {
            axis: side.axis(),
            ratio: clamp_ratio(DEFAULT_RATIO),
            first: Box::new(first),
            second: Box::new(second),
        };
        return true;
    }
    // Otherwise recurse into children.
    if let PaneNode::Split { first, second, .. } = node {
        if split_leaf(first, target, side, new_id, new_module) {
            return true;
        }
        if split_leaf(second, target, side, new_id, new_module) {
            return true;
        }
    }
    false
}

#[derive(Debug, PartialEq, Eq)]
enum CloseResult {
    Closed,
    NotFound,
    /// Target is the root leaf — can't close the last pane.
    IsRoot,
}

/// Close the target leaf. If found as a direct child of a split, replace
/// the split with the surviving sibling. If `node` IS the target leaf,
/// returns `IsRoot`. Records the new focus (first leaf of the surviving
/// subtree) in `new_focus`.
fn close_leaf(node: &mut PaneNode, target: PaneId, new_focus: &mut Option<PaneId>) -> CloseResult {
    // If `node` itself is the target leaf, it's the root — can't close.
    if node.is_leaf_with_id(target) {
        return CloseResult::IsRoot;
    }
    let PaneNode::Split { first, second, .. } = node else {
        return CloseResult::NotFound;
    };

    // If first is the target leaf, replace `*node` with `second`.
    if first.is_leaf_with_id(target) {
        // Move `second` out (replace with a placeholder that's about to be
        // dropped along with the rest of `*node`).
        let survivor = std::mem::replace(
            second.as_mut(),
            PaneNode::Leaf {
                id: PaneId(u64::MAX),
                module: ModuleId::Placeholder,
            },
        );
        *node = survivor;
        *new_focus = Some(first_leaf_id(node));
        return CloseResult::Closed;
    }
    // Same for second.
    if second.is_leaf_with_id(target) {
        let survivor = std::mem::replace(
            first.as_mut(),
            PaneNode::Leaf {
                id: PaneId(u64::MAX),
                module: ModuleId::Placeholder,
            },
        );
        *node = survivor;
        *new_focus = Some(first_leaf_id(node));
        return CloseResult::Closed;
    }

    // Recurse into children.
    if matches!(close_leaf(first, target, new_focus), CloseResult::Closed) {
        return CloseResult::Closed;
    }
    if matches!(close_leaf(second, target, new_focus), CloseResult::Closed) {
        return CloseResult::Closed;
    }
    // We don't propagate IsRoot from children — a child being the root is
    // impossible (we're at a non-root node). Defensive: treat as NotFound.
    CloseResult::NotFound
}

/// Change the module assigned to a leaf. Returns true if found.
fn set_module_leaf(node: &mut PaneNode, target: PaneId, module: ModuleId) -> bool {
    match node {
        PaneNode::Leaf { id, module: m } => {
            if *id == target {
                *m = module;
                true
            } else {
                false
            }
        }
        PaneNode::Split { first, second, .. } => {
            set_module_leaf(first, target, module) || set_module_leaf(second, target, module)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ResizeResult {
    Resized,
    NoAncestor,
}

/// Walk the path from root to leaf. At each Split, recurse first (to find
/// the DEEPEST matching ancestor — the one whose boundary is physically
/// adjacent to the focused pane). If no deeper ancestor matches, check
/// the current split.
fn resize_recursive(
    node: &mut PaneNode,
    path: &[ChildPos],
    dir: Direction,
    step: f32,
) -> ResizeResult {
    let PaneNode::Split {
        axis,
        ratio,
        first,
        second,
    } = node
    else {
        return ResizeResult::NoAncestor;
    };
    if path.is_empty() {
        return ResizeResult::NoAncestor;
    }
    let pos = path[0];
    let rest = &path[1..];

    // Recurse deeper first (find the deepest matching ancestor).
    if !rest.is_empty() {
        let child = match pos {
            ChildPos::First => first.as_mut(),
            ChildPos::Second => second.as_mut(),
        };
        let deeper = resize_recursive(child, rest, dir, step);
        if matches!(deeper, ResizeResult::Resized) {
            return deeper;
        }
    }

    // Check if THIS is a matching ancestor. The axis must match the
    // requested direction (Vertical for left/right, Horizontal for
    // up/down), and the focused pane must be on the side that lets the
    // boundary move in the requested direction.
    let axis_matches = match dir {
        Direction::Left | Direction::Right => *axis == SplitAxis::Vertical,
        Direction::Up | Direction::Down => *axis == SplitAxis::Horizontal,
    };
    let side_matches = match dir {
        // Growing left = shrinking the left sibling = focused pane is the
        // SECOND (right) child of a Vertical split; decrease ratio.
        Direction::Left => pos == ChildPos::Second,
        // Growing right = shrinking the right sibling = focused pane is
        // the FIRST (left) child; increase ratio.
        Direction::Right => pos == ChildPos::First,
        // Growing down = shrinking the bottom sibling = focused pane is
        // the FIRST (top) child of a Horizontal split; increase ratio.
        Direction::Down => pos == ChildPos::First,
        // Growing up = shrinking the top sibling = focused pane is the
        // SECOND (bottom) child; decrease ratio.
        Direction::Up => pos == ChildPos::Second,
    };
    if axis_matches && side_matches {
        let delta = if matches!(dir, Direction::Left | Direction::Up) {
            -step
        } else {
            step
        };
        *ratio = clamp_ratio(*ratio + delta);
        return ResizeResult::Resized;
    }
    ResizeResult::NoAncestor
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(id: u64, module: ModuleId) -> PaneNode {
        PaneNode::Leaf {
            id: PaneId(id),
            module,
        }
    }

    fn split(axis: SplitAxis, ratio: f32, first: PaneNode, second: PaneNode) -> PaneNode {
        PaneNode::Split {
            axis,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    /// `split_leaf` replaces the target leaf with a new Split.
    #[test]
    fn split_leaf_replaces_target() {
        let mut root = leaf(0, ModuleId::Artists);
        let ok = split_leaf(
            &mut root,
            PaneId(0),
            Side::Right,
            PaneId(1),
            ModuleId::Queue,
        );
        assert!(ok);
        match root {
            PaneNode::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                assert_eq!(axis, SplitAxis::Vertical);
                assert!((ratio - 0.5).abs() < 1e-6);
                assert!(first.is_leaf_with_id(PaneId(0)));
                assert!(second.is_leaf_with_id(PaneId(1)));
            }
            _ => panic!("expected Split"),
        }
    }

    /// Splitting left makes the new pane the FIRST child.
    #[test]
    fn split_left_new_is_first() {
        let mut root = leaf(0, ModuleId::Artists);
        split_leaf(&mut root, PaneId(0), Side::Left, PaneId(1), ModuleId::Queue);
        match root {
            PaneNode::Split {
                axis,
                first,
                second,
                ..
            } => {
                assert_eq!(axis, SplitAxis::Vertical);
                assert!(first.is_leaf_with_id(PaneId(1))); // new
                assert!(second.is_leaf_with_id(PaneId(0))); // old
            }
            _ => panic!("expected Split"),
        }
    }

    /// Splitting top makes the new pane the FIRST child (Horizontal axis).
    #[test]
    fn split_top_new_is_first() {
        let mut root = leaf(0, ModuleId::Artists);
        split_leaf(&mut root, PaneId(0), Side::Top, PaneId(1), ModuleId::Queue);
        match root {
            PaneNode::Split {
                axis,
                first,
                second,
                ..
            } => {
                assert_eq!(axis, SplitAxis::Horizontal);
                assert!(first.is_leaf_with_id(PaneId(1)));
                assert!(second.is_leaf_with_id(PaneId(0)));
            }
            _ => panic!("expected Split"),
        }
    }

    /// Splitting bottom makes the new pane the SECOND child (Horizontal).
    #[test]
    fn split_bottom_new_is_second() {
        let mut root = leaf(0, ModuleId::Artists);
        split_leaf(
            &mut root,
            PaneId(0),
            Side::Bottom,
            PaneId(1),
            ModuleId::Queue,
        );
        match root {
            PaneNode::Split {
                axis,
                first,
                second,
                ..
            } => {
                assert_eq!(axis, SplitAxis::Horizontal);
                assert!(first.is_leaf_with_id(PaneId(0)));
                assert!(second.is_leaf_with_id(PaneId(1)));
            }
            _ => panic!("expected Split"),
        }
    }

    /// `find_path` returns the path from root to the target leaf.
    #[test]
    fn find_path_returns_correct_descent() {
        let root = split(
            SplitAxis::Vertical,
            0.5,
            leaf(0, ModuleId::Artists),
            split(
                SplitAxis::Horizontal,
                0.5,
                leaf(1, ModuleId::Queue),
                leaf(2, ModuleId::Youtube),
            ),
        );
        assert_eq!(find_path(&root, PaneId(0)), Some(vec![ChildPos::First]));
        assert_eq!(
            find_path(&root, PaneId(2)),
            Some(vec![ChildPos::Second, ChildPos::Second])
        );
        assert_eq!(find_path(&root, PaneId(99)), None);
        // Root leaf (path is empty).
        let single = leaf(0, ModuleId::Artists);
        assert_eq!(find_path(&single, PaneId(0)), Some(vec![]));
    }

    /// `close_leaf` replaces the parent split with the surviving sibling.
    #[test]
    fn close_replaces_parent_with_sibling() {
        let mut root = split(
            SplitAxis::Vertical,
            0.5,
            leaf(0, ModuleId::Artists),
            leaf(1, ModuleId::Queue),
        );
        let mut new_focus = None;
        assert_eq!(
            close_leaf(&mut root, PaneId(0), &mut new_focus),
            CloseResult::Closed
        );
        assert!(root.is_leaf_with_id(PaneId(1)));
        assert_eq!(new_focus, Some(PaneId(1)));
    }

    /// Closing the root leaf returns `IsRoot`.
    #[test]
    fn close_root_refused() {
        let mut root = leaf(0, ModuleId::Artists);
        let mut new_focus = None;
        assert_eq!(
            close_leaf(&mut root, PaneId(0), &mut new_focus),
            CloseResult::IsRoot
        );
    }

    /// Closing a nested leaf collapses its immediate parent, not the
    /// whole tree.
    #[test]
    fn close_nested_collapses_immediate_parent() {
        // Tree: Split(V, Split(H, A, B), C)
        let mut root = split(
            SplitAxis::Vertical,
            0.5,
            split(
                SplitAxis::Horizontal,
                0.5,
                leaf(0, ModuleId::Artists),
                leaf(1, ModuleId::Queue),
            ),
            leaf(2, ModuleId::Youtube),
        );
        let mut new_focus = None;
        assert_eq!(
            close_leaf(&mut root, PaneId(0), &mut new_focus),
            CloseResult::Closed
        );
        // After: Split(V, B, C) — the inner Horizontal split is gone.
        match &root {
            PaneNode::Split {
                axis,
                first,
                second,
                ..
            } => {
                assert_eq!(*axis, SplitAxis::Vertical);
                assert!(first.is_leaf_with_id(PaneId(1)));
                assert!(second.is_leaf_with_id(PaneId(2)));
            }
            _ => panic!("expected Split"),
        }
    }

    /// `clamp_ratio` enforces [MIN_RATIO, MAX_RATIO].
    #[test]
    fn clamp_ratio_bounds() {
        assert_eq!(clamp_ratio(-1.0), MIN_RATIO);
        assert_eq!(clamp_ratio(0.0), MIN_RATIO);
        assert_eq!(clamp_ratio(0.5), 0.5);
        assert_eq!(clamp_ratio(1.0), MAX_RATIO);
        assert_eq!(clamp_ratio(2.0), MAX_RATIO);
    }

    /// No duplicate PaneId after repeated splits + closes.
    #[test]
    fn no_duplicate_pane_ids_after_mutations() {
        let mut ws = PaneWorkspace::new();
        // Split root three times — each split should get a fresh id.
        let id1 = ws.split(PaneId(0), Side::Right, ModuleId::Queue).unwrap();
        let id2 = ws.split(PaneId(0), Side::Right, ModuleId::Youtube).unwrap();
        let id3 = ws
            .split(PaneId(0), Side::Right, ModuleId::Playlists)
            .unwrap();
        let ids = leaf_ids(&ws.root);
        // All ids are unique.
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "duplicate ids found: {ids:?}");
        // The three new ids are 1, 2, 3.
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
        assert!(ids.contains(&id3));
        // Close one and split again — the new id must not collide.
        ws.close(id1).unwrap();
        let id4 = ws.split(PaneId(0), Side::Right, ModuleId::Artists).unwrap();
        let ids_after = leaf_ids(&ws.root);
        assert!(!ids_after.contains(&id1), "id {id1:?} was reused");
        assert!(ids_after.contains(&id4));
        // All still unique.
        let mut sorted = ids_after.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids_after.len());
    }

    /// Resize: growing the right pane leftward decreases the Vertical
    /// ancestor's ratio.
    #[test]
    fn resize_grow_left_decreases_ratio() {
        // Split(V, 0.5, A, B). Focused = B. Grow left.
        let mut root = split(
            SplitAxis::Vertical,
            0.5,
            leaf(0, ModuleId::Artists),
            leaf(1, ModuleId::Queue),
        );
        let path = find_path(&root, PaneId(1)).unwrap();
        assert_eq!(
            resize_recursive(&mut root, &path, Direction::Left, 0.02),
            ResizeResult::Resized
        );
        match root {
            PaneNode::Split { ratio, .. } => {
                assert!(ratio < 0.5, "ratio should have decreased: {ratio}");
            }
            _ => panic!("expected Split"),
        }
    }

    /// Resize: growing the left pane rightward increases the Vertical
    /// ancestor's ratio.
    #[test]
    fn resize_grow_right_increases_ratio() {
        let mut root = split(
            SplitAxis::Vertical,
            0.5,
            leaf(0, ModuleId::Artists),
            leaf(1, ModuleId::Queue),
        );
        let path = find_path(&root, PaneId(0)).unwrap();
        assert_eq!(
            resize_recursive(&mut root, &path, Direction::Right, 0.02),
            ResizeResult::Resized
        );
        match root {
            PaneNode::Split { ratio, .. } => {
                assert!(ratio > 0.5, "ratio should have increased: {ratio}");
            }
            _ => panic!("expected Split"),
        }
    }

    /// Resize: when there's no matching ancestor (e.g. growing the left
    /// pane further left), the operation is a no-op.
    #[test]
    fn resize_no_ancestor_is_noop() {
        let mut root = split(
            SplitAxis::Vertical,
            0.5,
            leaf(0, ModuleId::Artists),
            leaf(1, ModuleId::Queue),
        );
        // Focused = A (left). Grow left — no left sibling to shrink.
        let path = find_path(&root, PaneId(0)).unwrap();
        assert_eq!(
            resize_recursive(&mut root, &path, Direction::Left, 0.02),
            ResizeResult::NoAncestor
        );
        // Ratio unchanged.
        match root {
            PaneNode::Split { ratio, .. } => assert!((ratio - 0.5).abs() < 1e-6),
            _ => panic!("expected Split"),
        }
    }

    /// Resize: nested tree — the DEEPEST matching ancestor is adjusted.
    /// Split(V, 0.5, A, Split(H, 0.5, B, C)). Focused = C. Grow left.
    /// C's left boundary is B's right boundary (no V split between them),
    /// so the nearest V ancestor is the ROOT. Adjusting the root ratio
    /// grows C leftward by absorbing space from A.
    #[test]
    fn resize_uses_deepest_matching_ancestor() {
        let mut root = split(
            SplitAxis::Vertical,
            0.5,
            leaf(0, ModuleId::Artists),
            split(
                SplitAxis::Horizontal,
                0.5,
                leaf(1, ModuleId::Queue),
                leaf(2, ModuleId::Youtube),
            ),
        );
        let path = find_path(&root, PaneId(2)).unwrap();
        assert_eq!(path, vec![ChildPos::Second, ChildPos::Second]);
        assert_eq!(
            resize_recursive(&mut root, &path, Direction::Left, 0.05),
            ResizeResult::Resized
        );
        // The root V split's ratio should have decreased.
        match &root {
            PaneNode::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                assert_eq!(*axis, SplitAxis::Vertical);
                assert!(*ratio < 0.5, "root ratio should have decreased: {ratio}");
                assert!(first.is_leaf_with_id(PaneId(0)));
                // The inner H split should be unchanged.
                match second.as_ref() {
                    PaneNode::Split { axis, ratio, .. } => {
                        assert_eq!(*axis, SplitAxis::Horizontal);
                        assert!(
                            (ratio - 0.5).abs() < 1e-6,
                            "inner ratio should be unchanged"
                        );
                    }
                    _ => panic!("expected inner Split"),
                }
            }
            _ => panic!("expected root Split"),
        }
    }

    /// Resize clamps at MIN_RATIO / MAX_RATIO.
    #[test]
    fn resize_clamps() {
        let mut root = split(
            SplitAxis::Vertical,
            0.5,
            leaf(0, ModuleId::Artists),
            leaf(1, ModuleId::Queue),
        );
        // Grow B left repeatedly — A's share should clamp at MIN_RATIO.
        for _ in 0..100 {
            let path = find_path(&root, PaneId(1)).unwrap();
            let _ = resize_recursive(&mut root, &path, Direction::Left, 0.1);
        }
        match root {
            PaneNode::Split { ratio, .. } => assert_eq!(ratio, MIN_RATIO),
            _ => panic!("expected Split"),
        }
    }

    /// `PaneWorkspace::split` returns the new pane's id and focuses it.
    #[test]
    fn workspace_split_returns_and_focuses_new_id() {
        let mut ws = PaneWorkspace::new();
        let new = ws.split(PaneId(0), Side::Right, ModuleId::Queue).unwrap();
        assert_eq!(new, PaneId(1));
        assert_eq!(ws.focused_pane, PaneId(1));
        assert_eq!(ws.root.leaf_count(), 2);
    }

    /// `PaneWorkspace::close` on the root leaf returns `IsRoot`.
    #[test]
    fn workspace_close_root_refused() {
        let mut ws = PaneWorkspace::new();
        assert_eq!(ws.close(PaneId(0)), Err(CloseError::IsRoot));
        // After splitting, closing one of the two leaves succeeds.
        let new = ws.split(PaneId(0), Side::Right, ModuleId::Queue).unwrap();
        assert!(ws.close(new).is_ok());
        assert_eq!(ws.root.leaf_count(), 1);
    }

    /// `set_module` changes the module on a leaf.
    #[test]
    fn set_module_changes_leaf() {
        let mut ws = PaneWorkspace::new();
        assert!(ws.set_module(PaneId(0), ModuleId::Queue));
        match &ws.root {
            PaneNode::Leaf { module, .. } => assert_eq!(*module, ModuleId::Queue),
            _ => panic!("expected Leaf"),
        }
        // Unknown id → false.
        assert!(!ws.set_module(PaneId(99), ModuleId::Queue));
    }

    /// `is_active` is false for a fresh workspace, true after a split or
    /// entering edit mode.
    #[test]
    fn is_active_after_split_or_edit_mode() {
        let mut ws = PaneWorkspace::new();
        assert!(!ws.is_active(), "fresh workspace should not be active");
        ws.enter_edit_mode();
        assert!(ws.is_active(), "edit mode should activate");
        ws.exit_edit_mode();
        assert!(!ws.is_active());
        ws.split(PaneId(0), Side::Right, ModuleId::Queue).unwrap();
        assert!(ws.is_active(), "after split, workspace should be active");
        ws.exit_edit_mode();
        assert!(ws.is_active(), "still active: more than one pane");
    }

    /// `ModuleId::label` returns a non-empty label for each variant.
    #[test]
    fn module_id_labels() {
        assert_eq!(ModuleId::Artists.label(), "Artists");
        assert_eq!(ModuleId::Playlists.label(), "Playlists");
        assert_eq!(ModuleId::Queue.label(), "Queue");
        assert_eq!(ModuleId::Youtube.label(), "YouTube");
        assert_eq!(ModuleId::Placeholder.label(), "Placeholder");
    }

    /// `ModuleId::all` returns the modules in the picker order.
    #[test]
    fn module_id_all_returns_picker_order() {
        let all = ModuleId::all();
        assert_eq!(all[0], ModuleId::Artists);
        assert_eq!(all[4], ModuleId::NowPlaying);
        assert_eq!(all[5], ModuleId::YtHome);
        assert_eq!(all[11], ModuleId::YtCharts);
        assert_eq!(all[12], ModuleId::Placeholder);
        assert_eq!(all.len(), 13);
    }

    /// `Default::default()` for PaneWorkspace equals `new()`.
    #[test]
    fn default_equals_new() {
        let def: PaneWorkspace = Default::default();
        let new = PaneWorkspace::new();
        assert_eq!(def.focused_pane, new.focused_pane);
        assert_eq!(def.next_id, new.next_id);
        assert_eq!(def.root.leaf_count(), new.root.leaf_count());
        assert_eq!(def.mode, new.mode);
    }

    /// `split` on a non-existent target returns `Err(NotFound)` and does
    /// NOT burn a pane id (the next successful split reuses it).
    #[test]
    fn split_nonexistent_target_returns_err_and_does_not_burn_id() {
        let mut ws = PaneWorkspace::new();
        assert_eq!(ws.next_id, 1);
        let err = ws.split(PaneId(99), Side::Right, ModuleId::Queue);
        assert_eq!(err, Err(SplitError::NotFound));
        // Id not burned.
        assert_eq!(ws.next_id, 1);
        // Tree unchanged.
        assert_eq!(ws.root.leaf_count(), 1);
        // Next successful split reuses id 1.
        let new = ws.split(PaneId(0), Side::Right, ModuleId::Queue).unwrap();
        assert_eq!(new, PaneId(1));
    }

    /// `split_leaf` recurses into the second child when the target isn't
    /// in the first. Build a tree Split(V, A, B) and split B.
    #[test]
    fn split_leaf_recurses_into_second_child() {
        let mut root = split(
            SplitAxis::Vertical,
            0.5,
            leaf(0, ModuleId::Artists),
            leaf(1, ModuleId::Queue),
        );
        // Split the right child (id=1) on the right side.
        let ok = split_leaf(
            &mut root,
            PaneId(1),
            Side::Right,
            PaneId(2),
            ModuleId::Youtube,
        );
        assert!(ok);
        // Right child is now a Split.
        match &root {
            PaneNode::Split { second, .. } => match second.as_ref() {
                PaneNode::Split {
                    axis,
                    first,
                    second,
                    ..
                } => {
                    assert_eq!(*axis, SplitAxis::Vertical);
                    assert!(first.is_leaf_with_id(PaneId(1)));
                    assert!(second.is_leaf_with_id(PaneId(2)));
                }
                _ => panic!("expected inner Split"),
            },
            _ => panic!("expected outer Split"),
        }
    }

    /// `close` on a non-existent target returns `Err(NotFound)` (the
    /// target isn't the root, so it's not `IsRoot`).
    #[test]
    fn close_nonexistent_target_returns_not_found() {
        let mut ws = PaneWorkspace::new();
        // Need at least one split so close doesn't hit the IsRoot path.
        ws.split(PaneId(0), Side::Right, ModuleId::Queue).unwrap();
        let err = ws.close(PaneId(99));
        assert_eq!(err, Err(CloseError::NotFound));
    }

    /// `close_leaf` recurses into the second child when the target
    /// isn't in the first. Build Split(V, A, Split(H, B, C)) and close C.
    #[test]
    fn close_leaf_recurses_into_second_child() {
        let mut root = split(
            SplitAxis::Vertical,
            0.5,
            leaf(0, ModuleId::Artists),
            split(
                SplitAxis::Horizontal,
                0.5,
                leaf(1, ModuleId::Queue),
                leaf(2, ModuleId::Youtube),
            ),
        );
        let mut new_focus: Option<PaneId> = None;
        let res = close_leaf(&mut root, PaneId(2), &mut new_focus);
        assert_eq!(res, CloseResult::Closed);
        // The right child is now a single leaf (id=1).
        match &root {
            PaneNode::Split { second, .. } => {
                assert!(second.is_leaf_with_id(PaneId(1)));
            }
            _ => panic!("expected outer Split"),
        }
        assert_eq!(new_focus, Some(PaneId(1)));
    }

    /// `close_leaf` returns `NotFound` for a target that doesn't exist
    /// anywhere in the tree (recurses through both children, finds
    /// nothing).
    #[test]
    fn close_leaf_not_found_for_missing_target() {
        let mut root = split(
            SplitAxis::Vertical,
            0.5,
            leaf(0, ModuleId::Artists),
            leaf(1, ModuleId::Queue),
        );
        let mut new_focus: Option<PaneId> = None;
        let res = close_leaf(&mut root, PaneId(99), &mut new_focus);
        assert_eq!(res, CloseResult::NotFound);
        assert!(
            new_focus.is_none(),
            "new_focus should not be set on NotFound"
        );
    }

    /// `resize` returns `Err(NoAncestor)` when there's no matching
    /// ancestor for the direction (e.g. growing left in a Horizontal
    /// split).
    #[test]
    fn resize_no_matching_ancestor_returns_err() {
        let mut ws = PaneWorkspace::new();
        ws.split(PaneId(0), Side::Bottom, ModuleId::Queue).unwrap();
        // Horizontal split — left/right resize has no matching ancestor.
        let err = ws.resize(PaneId(0), Direction::Left, 0.05);
        assert_eq!(err, Err(ResizeError::NoAncestor));
        let err = ws.resize(PaneId(0), Direction::Right, 0.05);
        assert_eq!(err, Err(ResizeError::NoAncestor));
    }

    /// `resize` returns `Err(NotFound)` when the target isn't in the
    /// tree.
    #[test]
    fn resize_nonexistent_target_returns_not_found() {
        let mut ws = PaneWorkspace::new();
        let err = ws.resize(PaneId(99), Direction::Left, 0.05);
        assert_eq!(err, Err(ResizeError::NotFound));
    }

    /// `resize` for `Direction::Down` adjusts the ratio upward (focused
    /// pane is the FIRST child of a Horizontal split).
    #[test]
    fn resize_down_increases_ratio() {
        let mut root = split(
            SplitAxis::Horizontal,
            0.5,
            leaf(0, ModuleId::Artists),
            leaf(1, ModuleId::Queue),
        );
        let path = find_path(&root, PaneId(0)).unwrap();
        assert_eq!(
            resize_recursive(&mut root, &path, Direction::Down, 0.05),
            ResizeResult::Resized
        );
        match &root {
            PaneNode::Split { ratio, .. } => assert!(*ratio > 0.5, "ratio should grow: {ratio}"),
            _ => panic!("expected Split"),
        }
    }

    /// `resize` for `Direction::Up` adjusts the ratio downward (focused
    /// pane is the SECOND child of a Horizontal split).
    #[test]
    fn resize_up_decreases_ratio() {
        let mut root = split(
            SplitAxis::Horizontal,
            0.5,
            leaf(0, ModuleId::Artists),
            leaf(1, ModuleId::Queue),
        );
        let path = find_path(&root, PaneId(1)).unwrap();
        assert_eq!(
            resize_recursive(&mut root, &path, Direction::Up, 0.05),
            ResizeResult::Resized
        );
        match &root {
            PaneNode::Split { ratio, .. } => assert!(*ratio < 0.5, "ratio should shrink: {ratio}"),
            _ => panic!("expected Split"),
        }
    }

    /// `resize_recursive` on a leaf node (no split to adjust) returns
    /// `NoAncestor`.
    #[test]
    fn resize_recursive_on_leaf_returns_no_ancestor() {
        let mut root = leaf(0, ModuleId::Artists);
        let path: Vec<ChildPos> = vec![];
        assert_eq!(
            resize_recursive(&mut root, &path, Direction::Left, 0.05),
            ResizeResult::NoAncestor
        );
    }

    /// `resize_recursive` recurses into the FIRST child when the path
    /// starts with First. Tree: Split(V, Split(H, A, B), C). Focus on
    /// A (path = [First, First]). Resizing A down matches the inner H
    /// split — the deeper call returns Resized, we return at line 668.
    #[test]
    fn resize_recursive_recurses_into_first_child() {
        let mut root = split(
            SplitAxis::Vertical,
            0.5,
            split(
                SplitAxis::Horizontal,
                0.5,
                leaf(0, ModuleId::Artists),
                leaf(1, ModuleId::Queue),
            ),
            leaf(2, ModuleId::Youtube),
        );
        // Path to A (id=0) is [First, First].
        let path = find_path(&root, PaneId(0)).unwrap();
        assert_eq!(path, vec![ChildPos::First, ChildPos::First]);
        // Grow A down — matches the inner H split (A is the FIRST child,
        // so grow-down = increase ratio).
        assert_eq!(
            resize_recursive(&mut root, &path, Direction::Down, 0.05),
            ResizeResult::Resized
        );
        // The inner H split's ratio should have increased.
        match &root {
            PaneNode::Split { first, .. } => match first.as_ref() {
                PaneNode::Split { axis, ratio, .. } => {
                    assert_eq!(*axis, SplitAxis::Horizontal);
                    assert!(*ratio > 0.5, "inner ratio should have increased: {ratio}");
                }
                _ => panic!("expected inner Split"),
            },
            _ => panic!("expected outer Split"),
        }
    }

    /// `resize_recursive` recurses into the FIRST child, but when the
    /// deeper call returns NoAncestor (no matching axis deeper), we
    /// fall through to check the current split. Tree: Split(V,
    /// Split(H, A, B), C). Focus on A. Grow A left — the inner H split
    /// doesn't match (Left needs Vertical), so the deeper call returns
    /// NoAncestor. The outer V split doesn't match either (A is First,
    /// grow-left needs Second). So the whole thing returns NoAncestor.
    #[test]
    fn resize_recursive_first_child_deeper_no_ancestor_falls_through() {
        let mut root = split(
            SplitAxis::Vertical,
            0.5,
            split(
                SplitAxis::Horizontal,
                0.5,
                leaf(0, ModuleId::Artists),
                leaf(1, ModuleId::Queue),
            ),
            leaf(2, ModuleId::Youtube),
        );
        let path = find_path(&root, PaneId(0)).unwrap();
        // Grow A left — no matching ancestor anywhere.
        let res = resize_recursive(&mut root, &path, Direction::Left, 0.05);
        // The outer V split COULD match: A is First, grow-left needs
        // Second. So no — outer doesn't match. Result is NoAncestor.
        assert_eq!(res, ResizeResult::NoAncestor);
    }

    /// `move_focus` on a workspace returns false when there's no
    /// candidate in the direction (single pane).
    #[test]
    fn workspace_move_focus_no_candidate_returns_false() {
        let mut ws = PaneWorkspace::new();
        let panes = crate::tui::pane::layout::resolve_rects(
            &ws.root,
            ratatui::layout::Rect::new(0, 0, 100, 30),
        );
        assert!(!ws.move_focus(&panes, Direction::Left));
        assert!(!ws.move_focus(&panes, Direction::Right));
        assert!(!ws.move_focus(&panes, Direction::Up));
        assert!(!ws.move_focus(&panes, Direction::Down));
    }

    /// `move_focus` moves to the adjacent pane and returns true.
    #[test]
    fn workspace_move_focus_moves_to_adjacent_pane() {
        let mut ws = PaneWorkspace::new();
        ws.split(PaneId(0), Side::Right, ModuleId::Queue).unwrap();
        // Now focused on the right pane (id=1).
        let panes = crate::tui::pane::layout::resolve_rects(
            &ws.root,
            ratatui::layout::Rect::new(0, 0, 100, 30),
        );
        assert!(ws.move_focus(&panes, Direction::Left));
        assert_eq!(ws.focused_pane, PaneId(0));
    }

    /// `cycle_focus` returns false on a single-pane workspace (no cycle).
    #[test]
    fn workspace_cycle_focus_single_pane_returns_false() {
        let mut ws = PaneWorkspace::new();
        let panes = crate::tui::pane::layout::resolve_rects(
            &ws.root,
            ratatui::layout::Rect::new(0, 0, 100, 30),
        );
        assert!(!ws.cycle_focus(&panes, true));
        assert!(!ws.cycle_focus(&panes, false));
    }

    /// `cycle_focus` moves through the panes in tree order.
    #[test]
    fn workspace_cycle_focus_multi_pane() {
        let mut ws = PaneWorkspace::new();
        ws.split(PaneId(0), Side::Right, ModuleId::Queue).unwrap();
        // Focused on pane 1 (right).
        let panes = crate::tui::pane::layout::resolve_rects(
            &ws.root,
            ratatui::layout::Rect::new(0, 0, 100, 30),
        );
        assert!(ws.cycle_focus(&panes, true));
        assert_eq!(ws.focused_pane, PaneId(0));
        assert!(ws.cycle_focus(&panes, false));
        assert_eq!(ws.focused_pane, PaneId(1));
    }

    /// `apply_rectangle_selection` with a non-existent target pane
    /// returns false (no mutation, no id burn).
    #[test]
    fn apply_rectangle_selection_bad_target_returns_false() {
        let mut ws = PaneWorkspace::new();
        let next_id_before = ws.next_id;
        // Build a selection that targets a non-existent pane.
        let sel = crate::tui::pane::selection::RectangleSelection::new(PaneId(99));
        let ok = ws.apply_rectangle_selection(&sel, ModuleId::Queue);
        assert!(!ok, "should return false for bad target");
        // No id burn.
        assert_eq!(ws.next_id, next_id_before);
        // Tree unchanged.
        assert_eq!(ws.root.leaf_count(), 1);
    }

    /// `replace_leaf` replaces a leaf that's the second child of a
    /// split (covers the recursion-into-second branch).
    #[test]
    fn replace_leaf_replaces_second_child() {
        let mut root = split(
            SplitAxis::Vertical,
            0.5,
            leaf(0, ModuleId::Artists),
            leaf(1, ModuleId::Queue),
        );
        let replacement = leaf(99, ModuleId::Youtube);
        let ok = replace_leaf(&mut root, PaneId(1), replacement);
        assert!(ok);
        match &root {
            PaneNode::Split { second, .. } => assert!(second.is_leaf_with_id(PaneId(99))),
            _ => panic!("expected Split"),
        }
    }

    /// `replace_leaf` returns false for a target not in the tree.
    #[test]
    fn replace_leaf_returns_false_for_missing_target() {
        let mut root = leaf(0, ModuleId::Artists);
        let replacement = leaf(99, ModuleId::Youtube);
        assert!(!replace_leaf(&mut root, PaneId(5), replacement));
    }

    /// `replace_leaf` on a Split where NEITHER child contains the
    /// target returns false (covers the fall-through path through
    /// both children's false branches).
    #[test]
    fn replace_leaf_split_no_match_returns_false() {
        let mut root = split(
            SplitAxis::Vertical,
            0.5,
            leaf(0, ModuleId::Artists),
            leaf(1, ModuleId::Queue),
        );
        let replacement = leaf(99, ModuleId::Youtube);
        assert!(!replace_leaf(&mut root, PaneId(42), replacement));
        // Tree unchanged.
        assert!(
            root.is_leaf_with_id(PaneId(0))
                || matches!(&root, PaneNode::Split { first, .. } if first.is_leaf_with_id(PaneId(0)))
        );
    }

    /// `replace_leaf` replaces a leaf at the root (no split).
    #[test]
    fn replace_leaf_replaces_root() {
        let mut root = leaf(0, ModuleId::Artists);
        let replacement = leaf(99, ModuleId::Youtube);
        let ok = replace_leaf(&mut root, PaneId(0), replacement);
        assert!(ok);
        match &root {
            PaneNode::Leaf { id, .. } => assert_eq!(*id, PaneId(99)),
            _ => panic!("expected Leaf"),
        }
    }

    /// `replace_leaf` recurses into the first child when the target is
    /// nested (Split(V, Split(H, A, B), C)) and we replace B.
    #[test]
    fn replace_leaf_recurses_into_first_child() {
        let mut root = split(
            SplitAxis::Vertical,
            0.5,
            split(
                SplitAxis::Horizontal,
                0.5,
                leaf(0, ModuleId::Artists),
                leaf(1, ModuleId::Queue),
            ),
            leaf(2, ModuleId::Youtube),
        );
        let replacement = leaf(99, ModuleId::Placeholder);
        let ok = replace_leaf(&mut root, PaneId(1), replacement);
        assert!(ok);
        // Find leaf 99 in the tree.
        fn has_leaf(node: &PaneNode, id: PaneId) -> bool {
            match node {
                PaneNode::Leaf { id: lid, .. } => *lid == id,
                PaneNode::Split { first, second, .. } => {
                    has_leaf(first, id) || has_leaf(second, id)
                }
            }
        }
        assert!(
            has_leaf(&root, PaneId(99)),
            "replacement leaf 99 should be in the tree"
        );
    }
}
