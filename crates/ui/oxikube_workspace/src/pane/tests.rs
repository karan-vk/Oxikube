//! Pure tests of the pane-group snapshot over a dock-area layout tree (no `App`).

use super::*;
use oxikube_ui::dock::{InsertTarget, RootKind};

fn panel(n: u64) -> PanelId {
    PanelId::from_u64(n)
}

fn item(n: u64) -> EntityId {
    EntityId::from(n * 100)
}

/// Panels 1..=n map to items 100, 200, ...
fn items(n: u64) -> HashMap<PanelId, EntityId> {
    (1..=n).map(|i| (panel(i), item(i))).collect()
}

/// A centre tree with panel 1 in a single pane.
fn one_pane() -> (PaneTree, NodeId) {
    let mut tree = PaneTree::new(RootKind::Split);
    let root = tree.root().id();
    tree.insert_panel(
        panel(1),
        InsertTarget::Split {
            node: root,
            placement: Placement::Right,
            size: None,
        },
    );
    let pane = tree.find_panel_node(panel(1)).expect("pane");
    (tree, pane)
}

#[test]
fn empty_centre_has_no_panes() {
    let tree = PaneTree::new(RootKind::Split);
    let group = PaneGroup::from_tree(&tree, &items(0));
    assert!(group.is_empty());
    assert!(group.panes().is_empty());
}

#[test]
fn single_pane_is_reported_without_its_wrapping_split() {
    let (mut tree, pane) = one_pane();
    tree.insert_panel(
        panel(2),
        InsertTarget::Tabs {
            node: pane,
            ix: None,
            activate: true,
        },
    );
    let group = PaneGroup::from_tree(&tree, &items(2));
    let Some(Member::Pane(root)) = group.root() else {
        panic!("one pane is the root, got {:?}", group.root());
    };
    assert_eq!(root.items(), [item(1), item(2)]);
    assert_eq!(root.active_item(), Some(item(2)));
    assert_eq!(root.active_index(), Some(1));
    assert_eq!(
        group.pane_for_item(item(1)).map(Pane::id),
        Some(PaneId(pane))
    );
}

#[test]
fn right_then_down_splits_nest_axes_in_layout_order() {
    let (mut tree, left) = one_pane();
    tree.split(left, panel(2), SplitDirection::Right.placement(), None);
    let right = tree.find_panel_node(panel(2)).expect("right pane");
    tree.split(right, panel(3), SplitDirection::Down.placement(), None);

    let group = PaneGroup::from_tree(&tree, &items(3));
    let Some(Member::Axis(row)) = group.root() else {
        panic!("a split root, got {:?}", group.root());
    };
    assert_eq!(row.axis, Axis::Horizontal);
    assert_eq!(row.members.len(), 2);
    let Member::Axis(column) = &row.members[1] else {
        panic!("the right member is the vertical split");
    };
    assert_eq!(column.axis, Axis::Vertical);

    let order: Vec<_> = group.panes().iter().map(|p| p.items()[0]).collect();
    assert_eq!(order, [item(1), item(2), item(3)]);
}

#[test]
fn left_and_up_splits_put_the_new_pane_first() {
    let (mut tree, pane) = one_pane();
    tree.split(pane, panel(2), SplitDirection::Left.placement(), None);
    let group = PaneGroup::from_tree(&tree, &items(2));
    let order: Vec<_> = group.panes().iter().map(|p| p.items()[0]).collect();
    assert_eq!(order, [item(2), item(1)]);

    let (mut tree, pane) = one_pane();
    tree.split(pane, panel(2), SplitDirection::Up.placement(), None);
    let group = PaneGroup::from_tree(&tree, &items(2));
    let Some(Member::Axis(column)) = group.root() else {
        panic!("a split root");
    };
    assert_eq!(column.axis, Axis::Vertical);
    assert_eq!(group.panes()[0].items(), [item(2)]);
}

#[test]
fn panels_that_carry_no_item_are_left_out() {
    let (mut tree, pane) = one_pane();
    tree.insert_panel(
        panel(9),
        InsertTarget::Tabs {
            node: pane,
            ix: None,
            activate: true,
        },
    );
    let group = PaneGroup::from_tree(&tree, &items(1));
    let pane = group.panes()[0];
    assert_eq!(pane.items(), [item(1)]);
    // The displayed panel is not an item.
    assert_eq!(pane.active_item(), None);
}

#[test]
fn split_direction_axes() {
    assert_eq!(SplitDirection::Right.axis(), Axis::Horizontal);
    assert_eq!(SplitDirection::Left.axis(), Axis::Horizontal);
    assert_eq!(SplitDirection::Up.axis(), Axis::Vertical);
    assert_eq!(SplitDirection::Down.axis(), Axis::Vertical);
}
