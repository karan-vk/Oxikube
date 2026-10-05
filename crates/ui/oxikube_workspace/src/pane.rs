//! Panes and the pane group: the split layout of the centre area.
//!
//! Zed keeps the centre as a `PaneGroup` (a tree of horizontal and vertical splits) whose leaves
//! are `Pane`s (tab strips of items). Here the tree lives in gpui-component's `DockArea` (its
//! centre `PaneTree`, which normalises itself on every edit and keeps container ids stable), and
//! [`PaneGroup`] / [`Pane`] are read-only snapshots of it in terms of items. Edits go through the
//! [`Workspace`](crate::Workspace): `split_pane`, `move_item`, `close_item`...

use std::collections::HashMap;

use gpui::{Axis, EntityId};
use oxikube_ui::dock::{NodeId, PaneNode, PaneRef, PaneTree, PanelId, Placement};

/// Identifies a pane for as long as it exists. Stable across unrelated edits (splitting another
/// pane, moving items between other panes); a pane whose last item leaves is gone for good.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaneId(pub(crate) NodeId);

impl PaneId {
    pub(crate) fn node(self) -> NodeId {
        self.0
    }
}

/// One pane: a tab strip of items, one of them displayed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pane {
    id: PaneId,
    items: Vec<EntityId>,
    active_item: Option<EntityId>,
}

impl Pane {
    /// The pane's id.
    pub fn id(&self) -> PaneId {
        self.id
    }

    /// The items in tab order.
    pub fn items(&self) -> &[EntityId] {
        &self.items
    }

    /// The displayed item.
    pub fn active_item(&self) -> Option<EntityId> {
        self.active_item
    }

    /// The index of the displayed item.
    pub fn active_index(&self) -> Option<usize> {
        self.active_item.and_then(|item| self.index_of(item))
    }

    /// The tab index of `item`, if this pane holds it.
    pub fn index_of(&self, item: EntityId) -> Option<usize> {
        self.items.iter().position(|held| *held == item)
    }

    /// Number of items.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the pane holds no item (only possible while a non-item panel sits in it).
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// The direction of a split, relative to the pane being split.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SplitDirection {
    /// The new pane goes above.
    Up,
    /// The new pane goes below.
    Down,
    /// The new pane goes to the left.
    Left,
    /// The new pane goes to the right.
    Right,
}

impl SplitDirection {
    /// The axis the panes are laid out along after the split.
    pub fn axis(self) -> Axis {
        match self {
            SplitDirection::Up | SplitDirection::Down => Axis::Vertical,
            SplitDirection::Left | SplitDirection::Right => Axis::Horizontal,
        }
    }

    pub(crate) fn placement(self) -> Placement {
        match self {
            SplitDirection::Up => Placement::Top,
            SplitDirection::Down => Placement::Bottom,
            SplitDirection::Left => Placement::Left,
            SplitDirection::Right => Placement::Right,
        }
    }
}

/// A node of the pane group: a pane or a split.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Member {
    /// A leaf.
    Pane(Pane),
    /// A split of two or more members along one axis.
    Axis(PaneAxis),
}

/// A split: its members side by side (horizontal) or stacked (vertical), in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneAxis {
    /// The direction members are laid out along.
    pub axis: Axis,
    /// The members, left to right or top to bottom.
    pub members: Vec<Member>,
}

/// A snapshot of the centre layout.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaneGroup {
    root: Option<Member>,
}

impl PaneGroup {
    /// Reads the dock area's centre tree. `items` maps dock panels to the items they carry;
    /// panels that carry no item (a side panel dragged into the centre) are left out.
    pub(crate) fn from_tree(tree: &PaneTree, items: &HashMap<PanelId, EntityId>) -> Self {
        Self {
            root: member(tree.root(), items),
        }
    }

    /// The root member, `None` when the centre is empty.
    pub fn root(&self) -> Option<&Member> {
        self.root.as_ref()
    }

    /// Every pane, in layout order (left to right, top to bottom).
    pub fn panes(&self) -> Vec<&Pane> {
        let mut panes = Vec::new();
        if let Some(root) = &self.root {
            collect(root, &mut panes);
        }
        panes
    }

    /// The pane with `id`.
    pub fn pane(&self, id: PaneId) -> Option<&Pane> {
        self.panes().into_iter().find(|pane| pane.id == id)
    }

    /// The pane holding `item`.
    pub fn pane_for_item(&self, item: EntityId) -> Option<&Pane> {
        self.panes()
            .into_iter()
            .find(|pane| pane.items.contains(&item))
    }

    /// Whether the centre holds no pane.
    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }
}

fn member(node: &PaneNode, items: &HashMap<PanelId, EntityId>) -> Option<Member> {
    match node.kind() {
        PaneRef::Tabs { panels, active_ix } => Some(Member::Pane(Pane {
            id: PaneId(node.id()),
            items: panels
                .iter()
                .filter_map(|p| items.get(p).copied())
                .collect(),
            active_item: panels.get(active_ix).and_then(|p| items.get(p).copied()),
        })),
        PaneRef::Split { axis, children, .. } => {
            let mut members: Vec<Member> = children
                .iter()
                .filter_map(|child| member(child, items))
                .collect();
            // The centre's root is always a split, even around a single pane: report the pane.
            match members.len() {
                0 => None,
                1 => members.pop(),
                _ => Some(Member::Axis(PaneAxis { axis, members })),
            }
        }
    }
}

fn collect<'a>(member: &'a Member, panes: &mut Vec<&'a Pane>) {
    match member {
        Member::Pane(pane) => panes.push(pane),
        Member::Axis(axis) => axis.members.iter().for_each(|m| collect(m, panes)),
    }
}

#[cfg(test)]
mod tests;
