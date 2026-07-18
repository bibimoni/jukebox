//! Serializable DTOs for `state.rs`.
//!
//! `PaneNode`, `PaneId`, `ModuleId`, and `SplitAxis` derive Serialize +
//! Deserialize directly (they're plain data). `PaneWorkspace` is not
//! serialized as-is because it owns the `next_id` counter — we serialize
//! a DTO that includes it so deserialization restores the counter (no
//! id reuse after a restart).
//!
//! `UiMode` is intentionally NOT persisted — the app always starts in
//! Normal mode. Pane-edit state is transient.

use serde::{Deserialize, Serialize};

use crate::tui::pane::model::{PaneId, PaneNode, PaneWorkspace, UiMode};

/// Serializable snapshot of a `PaneWorkspace`. Persisted as a field of
/// `state::LayoutState` (alongside the other UI prefs). Loaded on
/// startup, saved on exit.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaneWorkspaceDto {
    pub root: PaneNode,
    pub focused_pane: PaneId,
    pub next_id: u64,
}

impl From<&PaneWorkspace> for PaneWorkspaceDto {
    fn from(ws: &PaneWorkspace) -> Self {
        Self {
            root: ws.root.clone(),
            focused_pane: ws.focused_pane,
            next_id: ws.next_id,
        }
    }
}

impl From<PaneWorkspaceDto> for PaneWorkspace {
    fn from(dto: PaneWorkspaceDto) -> Self {
        // If the DTO is corrupt (focused_pane not in tree, next_id collides
        // with existing ids), fall back to safe defaults rather than
        // panicking. State is ephemeral UI prefs — losing it is
        // acceptable; crashing on launch is not.
        let focused_pane = if contains_leaf(&dto.root, dto.focused_pane) {
            dto.focused_pane
        } else {
            first_leaf_id(&dto.root)
        };
        let next_id = dto.next_id.max(max_leaf_id(&dto.root).0 + 1);
        Self {
            root: dto.root,
            focused_pane,
            mode: UiMode::Normal,
            next_id,
        }
    }
}

fn contains_leaf(node: &PaneNode, target: PaneId) -> bool {
    match node {
        PaneNode::Leaf { id, .. } => *id == target,
        PaneNode::Split { first, second, .. } => {
            contains_leaf(first, target) || contains_leaf(second, target)
        }
    }
}

fn first_leaf_id(node: &PaneNode) -> PaneId {
    match node {
        PaneNode::Leaf { id, .. } => *id,
        PaneNode::Split { first, .. } => first_leaf_id(first),
    }
}

fn max_leaf_id(node: &PaneNode) -> PaneId {
    match node {
        PaneNode::Leaf { id, .. } => *id,
        PaneNode::Split { first, second, .. } => {
            std::cmp::max(max_leaf_id(first), max_leaf_id(second))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::pane::model::{ModuleId, PaneWorkspace, Side};

    /// Round-trip: serialize a workspace to JSON, deserialize, verify
    /// the in-memory struct matches.
    #[test]
    fn round_trip_single_pane() {
        let ws = PaneWorkspace::new();
        let dto = PaneWorkspaceDto::from(&ws);
        let json = serde_json::to_string(&dto).unwrap();
        let restored: PaneWorkspaceDto = serde_json::from_str(&json).unwrap();
        let ws2: PaneWorkspace = restored.into();
        assert_eq!(ws.focused_pane, ws2.focused_pane);
        assert_eq!(ws.next_id, ws2.next_id);
        assert_eq!(ws.root.leaf_count(), ws2.root.leaf_count());
        assert_eq!(ws.mode, ws2.mode);
    }

    /// Round-trip with a complex nested tree.
    #[test]
    fn round_trip_nested_tree() {
        let mut ws = PaneWorkspace::new();
        // Build: Split(V, 0.5, A, Split(H, 0.3, B, C))
        let _ = ws.split(PaneId(0), Side::Right, ModuleId::Queue);
        // Focused is now the right pane (id=1). Split it horizontally.
        let _ = ws.split(PaneId(1), Side::Bottom, ModuleId::Youtube);
        // Close the middle pane to test collapse preservation.
        // Tree should now be: Split(V, 0.5, A, Split(H, 0.5, ?, C))
        let dto = PaneWorkspaceDto::from(&ws);
        let json = serde_json::to_string(&dto).unwrap();
        let restored: PaneWorkspace = serde_json::from_str::<PaneWorkspaceDto>(&json)
            .unwrap()
            .into();
        assert_eq!(restored.root.leaf_count(), ws.root.leaf_count());
        assert_eq!(restored.focused_pane, ws.focused_pane);
        assert_eq!(restored.next_id, ws.next_id);
    }

    /// Corrupt DTO (focused_pane not in tree) falls back to the first leaf
    /// rather than panicking.
    #[test]
    fn corrupt_focused_pane_falls_back() {
        let dto = PaneWorkspaceDto {
            root: PaneNode::Leaf {
                id: PaneId(0),
                module: ModuleId::Artists,
            },
            focused_pane: PaneId(99), // not in tree
            next_id: 1,
        };
        let ws: PaneWorkspace = dto.into();
        assert_eq!(ws.focused_pane, PaneId(0), "should fall back to first leaf");
    }

    /// Corrupt DTO (next_id collides with existing ids) is repaired so
    /// the next split gets a fresh id.
    #[test]
    fn corrupt_next_id_repaired() {
        let dto = PaneWorkspaceDto {
            root: PaneNode::Leaf {
                id: PaneId(5),
                module: ModuleId::Artists,
            },
            focused_pane: PaneId(5),
            next_id: 1, // collides: 1 < 5+1
        };
        let ws: PaneWorkspace = dto.into();
        assert_eq!(ws.next_id, 6, "next_id should be max+1");
    }

    /// `UiMode` is not in the DTO — restored workspace is always in
    /// Normal mode.
    #[test]
    fn restored_workspace_is_normal_mode() {
        let mut ws = PaneWorkspace::new();
        ws.enter_edit_mode();
        let dto = PaneWorkspaceDto::from(&ws);
        let restored: PaneWorkspace = dto.into();
        assert_eq!(restored.mode, UiMode::Normal);
    }

    /// A corrupt DTO with a Split root and a focused_pane not in the
    /// tree falls back to the first leaf (covers the Split branch of
    /// `first_leaf_id`).
    #[test]
    fn corrupt_dto_with_split_root_falls_back_to_first_leaf() {
        use crate::tui::pane::model::{PaneNode, SplitAxis};
        let dto = PaneWorkspaceDto {
            root: PaneNode::Split {
                axis: SplitAxis::Vertical,
                ratio: 0.5,
                first: Box::new(PaneNode::Leaf {
                    id: PaneId(0),
                    module: ModuleId::Artists,
                }),
                second: Box::new(PaneNode::Leaf {
                    id: PaneId(1),
                    module: ModuleId::Queue,
                }),
            },
            focused_pane: PaneId(99), // not in tree
            next_id: 5,
        };
        let ws: PaneWorkspace = dto.into();
        // First leaf of the split is PaneId(0).
        assert_eq!(ws.focused_pane, PaneId(0));
        assert_eq!(ws.root.leaf_count(), 2);
    }

    /// A DTO with a Split root and a valid focused_pane keeps the
    /// focused_pane (covers the Split branch of `contains_leaf`).
    #[test]
    fn split_root_with_valid_focused_pane_preserved() {
        use crate::tui::pane::model::{PaneNode, SplitAxis};
        let dto = PaneWorkspaceDto {
            root: PaneNode::Split {
                axis: SplitAxis::Vertical,
                ratio: 0.5,
                first: Box::new(PaneNode::Leaf {
                    id: PaneId(0),
                    module: ModuleId::Artists,
                }),
                second: Box::new(PaneNode::Leaf {
                    id: PaneId(1),
                    module: ModuleId::Queue,
                }),
            },
            focused_pane: PaneId(1),
            next_id: 5,
        };
        let ws: PaneWorkspace = dto.into();
        assert_eq!(ws.focused_pane, PaneId(1));
        // max_leaf_id is 1 → next_id = max(5, 1+1) = 5.
        assert_eq!(ws.next_id, 5);
    }

    /// `max_leaf_id` recurses through both children of a split.
    #[test]
    fn max_leaf_id_recurses_through_split() {
        use crate::tui::pane::model::{PaneNode, SplitAxis};
        let root = PaneNode::Split {
            axis: SplitAxis::Horizontal,
            ratio: 0.5,
            first: Box::new(PaneNode::Leaf {
                id: PaneId(3),
                module: ModuleId::Artists,
            }),
            second: Box::new(PaneNode::Split {
                axis: SplitAxis::Vertical,
                ratio: 0.5,
                first: Box::new(PaneNode::Leaf {
                    id: PaneId(7),
                    module: ModuleId::Queue,
                }),
                second: Box::new(PaneNode::Leaf {
                    id: PaneId(2),
                    module: ModuleId::Youtube,
                }),
            }),
        };
        // max leaf id is 7 (in the nested right split).
        assert_eq!(max_leaf_id(&root), PaneId(7));
    }
}
