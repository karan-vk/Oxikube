//! Pure edits of a saved [`PanelState`] tree: dropping item leaves, and the groups and splits
//! that become empty, while keeping tab indices and split sizes consistent.

use oxikube_ui::dock::{PanelInfo, PanelState};

use crate::item::ITEM_PANEL_NAME;

/// What an item tab was saved as: the kind it rebuilds under and its state.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemDescriptor {
    /// The item's [`Item::serialized_kind`](crate::Item::serialized_kind).
    pub kind: String,
    /// The item's [`Item::serialize`](crate::Item::serialize) output.
    pub state: serde_json::Value,
}

/// The descriptor of an item-tab leaf; `None` for any other panel state, or an item tab that was
/// saved without a kind (an item that cannot be rebuilt).
pub fn item_descriptor(state: &PanelState) -> Option<ItemDescriptor> {
    if state.panel_name != ITEM_PANEL_NAME {
        return None;
    }
    let PanelInfo::Panel(info) = &state.info else {
        return None;
    };
    Some(ItemDescriptor {
        kind: info.get("kind")?.as_str()?.to_owned(),
        state: info.get("state")?.clone(),
    })
}

/// Which tab a group displays after some of its tabs were dropped: the same one when it
/// survived, else the tab now at the old position (clamped).
pub(crate) fn surviving_active(saved_active: usize, survivors: &[usize]) -> usize {
    match survivors.iter().position(|&old| old == saved_active) {
        Some(new) => new,
        None => survivors
            .iter()
            .position(|&old| old > saved_active)
            .unwrap_or_else(|| survivors.len().saturating_sub(1)),
    }
}

/// Removes every leaf of `state` that `keep` rejects, then every tab group and split left empty,
/// fixing active indices and split sizes. Returns whether anything is left.
pub fn prune(state: &mut PanelState, keep: &mut impl FnMut(&PanelState) -> bool) -> bool {
    match state.info.clone() {
        PanelInfo::Tabs { active_index } => {
            let survivors: Vec<usize> = (0..state.children.len())
                .filter(|&ix| keep(&state.children[ix]))
                .collect();
            let mut ix = 0;
            state.children.retain(|_| {
                ix += 1;
                survivors.contains(&(ix - 1))
            });
            state.info = PanelInfo::tabs(surviving_active(active_index, &survivors));
            !state.children.is_empty()
        }
        PanelInfo::Stack { sizes, axis } => {
            let mut kept_sizes = Vec::new();
            let mut ix = 0;
            state.children.retain_mut(|child| {
                let size = sizes.get(ix).copied();
                ix += 1;
                let alive = prune(child, keep);
                if alive {
                    kept_sizes.extend(size);
                }
                alive
            });
            // A size list that did not match its children is kept as it was found.
            if kept_sizes.len() == state.children.len() {
                state.info = PanelInfo::Stack {
                    sizes: kept_sizes,
                    axis,
                };
            }
            !state.children.is_empty()
        }
        // A bare leaf where a container belongs.
        PanelInfo::Panel(_) => keep(state),
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Axis, px};
    use serde_json::json;

    use super::*;

    fn item(kind: Option<&str>, n: i32) -> PanelState {
        let mut leaf = PanelState::new(ITEM_PANEL_NAME);
        if let Some(kind) = kind {
            leaf.info = PanelInfo::panel(json!({ "kind": kind, "state": n }));
        }
        leaf
    }

    fn tabs(active: usize, children: Vec<PanelState>) -> PanelState {
        PanelState {
            panel_name: "TabPanel".into(),
            children,
            info: PanelInfo::tabs(active),
        }
    }

    fn stack(sizes: &[f32], children: Vec<PanelState>) -> PanelState {
        PanelState {
            panel_name: "StackPanel".into(),
            children,
            info: PanelInfo::stack(sizes.iter().map(|s| px(*s)).collect(), Axis::Horizontal),
        }
    }

    fn known(state: &PanelState) -> bool {
        item_descriptor(state).is_some_and(|d| d.kind == "known")
    }

    #[test]
    fn descriptor_needs_a_kind_and_a_state() {
        assert_eq!(
            item_descriptor(&item(Some("pods"), 3)),
            Some(ItemDescriptor {
                kind: "pods".into(),
                state: json!(3)
            })
        );
        assert_eq!(item_descriptor(&item(None, 0)), None);
        assert_eq!(item_descriptor(&PanelState::new("Other")), None);
        let mut half = PanelState::new(ITEM_PANEL_NAME);
        half.info = PanelInfo::panel(json!({ "kind": "pods" }));
        assert_eq!(item_descriptor(&half), None);
    }

    #[test]
    fn drops_rejected_tabs_and_keeps_the_displayed_one() {
        let mut group = tabs(
            2,
            vec![
                item(Some("known"), 0),
                item(Some("gone"), 1),
                item(Some("known"), 2),
            ],
        );
        assert!(prune(&mut group, &mut known));
        assert_eq!(group.children.len(), 2);
        assert_eq!(group.info.active_index(), Some(1), "still the third tab");
    }

    #[test]
    fn a_dropped_displayed_tab_hands_over_to_its_neighbour() {
        let mut group = tabs(
            1,
            vec![
                item(Some("known"), 0),
                item(Some("gone"), 1),
                item(Some("known"), 2),
            ],
        );
        prune(&mut group, &mut known);
        assert_eq!(group.info.active_index(), Some(1), "the tab after it");

        let mut last = tabs(
            2,
            vec![
                item(Some("known"), 0),
                item(Some("known"), 1),
                item(Some("gone"), 2),
            ],
        );
        prune(&mut last, &mut known);
        assert_eq!(last.info.active_index(), Some(1), "clamped to the last tab");
    }

    #[test]
    fn empty_groups_vanish_and_split_sizes_stay_aligned() {
        let mut root = stack(
            &[100., 200., 300.],
            vec![
                tabs(0, vec![item(Some("known"), 0)]),
                tabs(0, vec![item(Some("gone"), 1)]),
                tabs(0, vec![item(Some("known"), 2)]),
            ],
        );
        assert!(prune(&mut root, &mut known));
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.info.sizes(), Some(&vec![px(100.), px(300.)]));
    }

    #[test]
    fn nothing_left_reports_empty() {
        let mut root = stack(
            &[100.],
            vec![tabs(0, vec![item(Some("gone"), 0), item(None, 1)])],
        );
        assert!(!prune(&mut root, &mut known));
    }
}
