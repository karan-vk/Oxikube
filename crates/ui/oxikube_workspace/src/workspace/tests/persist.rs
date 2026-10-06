//! Capturing the layout and restoring a saved one (E05-S05): split workspaces round trip,
//! unknown items are skipped, docks come back with their size, visibility and tab.

use gpui::{Axis, Entity, px};
use oxikube_ui::{
    UiScale, Unscaled,
    dock::{DockAreaState, DockState, PanelInfo, PanelState},
    set_ui_scale,
};
use serde_json::json;

use super::*;
use crate::{
    item::ITEM_PANEL_NAME,
    pane::{Member, SplitDirection},
    persistence::{
        LAYOUT_SCHEMA_VERSION, RestoreReport, SerializedWorkspace, SkipReason, SkippedItem,
    },
    test_support::TestPanel,
};

/// A second window with an empty workspace, in the same app (so builders are registered).
fn second_workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, VisualTestContext) {
    let mut workspace = None;
    let window = cx.add_window(|window, cx| {
        let entity = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.update(|window, _| window.activate_window());
    vcx.run_until_parked();
    (workspace.expect("the window was built"), vcx)
}

/// The centre as nested titles: `[a b]*1` is a pane showing tab 1 of two.
fn shape(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> String {
    fn member(
        ws: &Entity<Workspace>,
        vcx: &mut VisualTestContext,
        node: &Member,
        out: &mut String,
    ) {
        match node {
            Member::Pane(pane) => {
                let titles = titles(ws, vcx, pane).join(" ");
                out.push_str(&format!("[{titles}]*{}", pane.active_index().unwrap_or(0)));
            }
            Member::Axis(axis) => {
                out.push(if axis.axis == Axis::Horizontal {
                    '('
                } else {
                    '{'
                });
                for (ix, child) in axis.members.iter().enumerate() {
                    if ix > 0 {
                        out.push(' ');
                    }
                    member(ws, vcx, child, out);
                }
                out.push(if axis.axis == Axis::Horizontal {
                    ')'
                } else {
                    '}'
                });
            }
        }
    }
    let group = vcx.update(|_, cx| ws.read(cx).pane_group(cx));
    let mut out = String::new();
    if let Some(root) = group.root() {
        member(ws, vcx, root, &mut out);
    }
    out
}

fn serialize(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> SerializedWorkspace {
    vcx.update(|_, cx| ws.read(cx).serialize_layout(cx))
}

fn restore(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    saved: &SerializedWorkspace,
) -> RestoreReport {
    let report =
        vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.restore_layout(saved, window, cx)));
    vcx.run_until_parked();
    report
}

/// Builds `(a b) / c` style layouts: a and b share the first pane, c is split to the right and d
/// below c.
fn open_split_workspace(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) {
    open(ws, vcx, "a");
    let b = open(ws, vcx, "b");
    let c = open(ws, vcx, "c");
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.move_item_to_split(c, SplitDirection::Right, window, cx)
        })
    })
    .expect("c splits right");
    vcx.run_until_parked();
    let d = open(ws, vcx, "d");
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.move_item_to_split(d, SplitDirection::Down, window, cx)
        })
    });
    vcx.run_until_parked();
    // Back to tab "a" in the first pane.
    let first = panes(ws, vcx)[0].items()[0];
    vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.activate_item(first, true, window, cx)));
    vcx.run_until_parked();
    let _ = b;
}

#[gpui::test]
fn a_split_workspace_round_trips(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open_split_workspace(&ws, &mut vcx);
    let before = shape(&ws, &mut vcx);
    assert_eq!(
        before, "([a b]*0 {[c]*0 [d]*0})",
        "the fixture is a split layout"
    );
    let saved = serialize(&ws, &mut vcx);
    assert_eq!(saved.version, LAYOUT_SCHEMA_VERSION);
    assert_eq!(saved.active_pane, Some(0));

    // Through JSON, as the store does.
    let saved = SerializedWorkspace::from_json(saved.to_json()).expect("reads back");

    let (fresh, mut fresh_vcx) = second_workspace(cx);
    let report = restore(&fresh, &mut fresh_vcx, &saved);
    assert_eq!(report.restored_items, 4);
    assert!(report.skipped_items.is_empty());
    assert!(report.centre_restored && !report.centre_kept);
    assert_eq!(shape(&fresh, &mut fresh_vcx), before);
    assert_eq!(active_pane(&fresh, &mut fresh_vcx).items().len(), 2);
    assert!(
        bounds(&mut fresh_vcx, "item-a").is_some(),
        "the displayed item is drawn"
    );
    assert!(
        bounds(&mut fresh_vcx, "item-b").is_none(),
        "the inactive tab is not"
    );

    // Restored items are real open items: they close, reopen and re-serialise.
    let again = serialize(&fresh, &mut fresh_vcx);
    assert_eq!(again.dock_area.center, saved.dock_area.center);
}

#[gpui::test]
fn the_displayed_tab_and_active_pane_come_back(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open_split_workspace(&ws, &mut vcx);
    // Make the bottom-right pane (d) the active one, and tab b the first pane's displayed tab.
    let b = panes(&ws, &mut vcx)[0].items()[1];
    vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.activate_item(b, true, window, cx)));
    let d_pane = panes(&ws, &mut vcx)[2].id();
    vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.activate_pane(d_pane, window, cx)));
    vcx.run_until_parked();
    let saved = serialize(&ws, &mut vcx);
    assert_eq!(saved.active_pane, Some(2));

    let (fresh, mut fresh_vcx) = second_workspace(cx);
    restore(&fresh, &mut fresh_vcx, &saved);
    assert_eq!(shape(&fresh, &mut fresh_vcx), "([a b]*1 {[c]*0 [d]*0})");
    let active = active_pane(&fresh, &mut fresh_vcx);
    assert_eq!(titles(&fresh, &mut fresh_vcx, &active), ["d"]);
    let item = active.active_item().expect("an active item");
    assert!(item_focused(&fresh, &mut fresh_vcx, item), "and focused");
}

/// A saved center of one tab group holding the given `(kind, state)` items.
fn saved_with(items: &[(&str, serde_json::Value)], active: usize) -> SerializedWorkspace {
    let mut tabs = PanelState::new("TabPanel");
    tabs.info = PanelInfo::tabs(active);
    for (kind, state) in items {
        let mut leaf = PanelState::new(ITEM_PANEL_NAME);
        leaf.info = PanelInfo::panel(json!({ "kind": kind, "state": state }));
        tabs.add_child(leaf);
    }
    let mut root = PanelState::new("StackPanel");
    root.info = PanelInfo::stack(vec![px(100.)], Axis::Horizontal);
    root.add_child(tabs);
    SerializedWorkspace {
        version: LAYOUT_SCHEMA_VERSION,
        window: None,
        active_pane: Some(0),
        dock_area: DockAreaState {
            center: root,
            ..Default::default()
        },
    }
}

#[gpui::test]
fn an_unknown_item_kind_is_skipped_and_the_rest_still_opens(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let kind = TestItem::KIND;
    let saved = saved_with(
        &[
            (kind, json!("first")),
            ("some_crate::Removed", json!({"anything": 1})),
            (kind, json!("third")),
            // A registered kind whose builder declines this state (it wants a string).
            (kind, json!(42)),
        ],
        2,
    );
    let report = restore(&ws, &mut vcx, &saved);
    assert_eq!(report.restored_items, 2);
    assert_eq!(
        report.skipped_items,
        [
            SkippedItem {
                kind: "some_crate::Removed".into(),
                reason: SkipReason::UnknownKind
            },
            SkippedItem {
                kind: kind.into(),
                reason: SkipReason::Declined
            },
        ]
    );
    let pane = active_pane(&ws, &mut vcx);
    assert_eq!(titles(&ws, &mut vcx, &pane), ["first", "third"]);
    assert_eq!(
        pane.active_index(),
        Some(1),
        "'third' was displayed and still is"
    );
    assert!(bounds(&mut vcx, "item-third").is_some());
}

#[gpui::test]
fn the_active_pane_follows_its_item_when_earlier_panes_are_skipped(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let kind = TestItem::KIND;
    // Panes A, B (an unregistered kind only) and C, saved with C active.
    let mut saved = saved_with(&[(kind, json!("a"))], 0);
    for (kind, title) in [("some_crate::Removed", json!(null)), (kind, json!("c"))] {
        let mut tabs = PanelState::new("TabPanel");
        tabs.info = PanelInfo::tabs(0);
        let mut leaf = PanelState::new(ITEM_PANEL_NAME);
        leaf.info = PanelInfo::panel(json!({ "kind": kind, "state": title }));
        tabs.add_child(leaf);
        saved.dock_area.center.add_child(tabs);
    }
    saved.dock_area.center.info = PanelInfo::stack(vec![px(100.); 3], Axis::Horizontal);
    saved.active_pane = Some(2);
    let report = restore(&ws, &mut vcx, &saved);
    assert_eq!(report.restored_items, 2);
    assert_eq!(shape(&ws, &mut vcx), "([a]*0 [c]*0)");
    let active = active_pane(&ws, &mut vcx);
    assert_eq!(titles(&ws, &mut vcx, &active), ["c"]);

    // Skipped panes before the active one shift its index, they do not change which pane it is.
    let (fresh, mut fresh_vcx) = second_workspace(cx);
    saved.active_pane = Some(0);
    restore(&fresh, &mut fresh_vcx, &saved);
    let active = active_pane(&fresh, &mut fresh_vcx);
    assert_eq!(titles(&fresh, &mut fresh_vcx, &active), ["a"]);
}

#[gpui::test]
fn a_layout_of_only_unknown_items_leaves_the_workspace_as_it_was(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let saved = saved_with(&[("some_crate::Removed", json!(null))], 0);
    let report = restore(&ws, &mut vcx, &saved);
    assert_eq!(report.restored_items, 0);
    assert_eq!(report.skipped_items.len(), 1);
    assert!(!report.centre_restored);
    assert!(panes(&ws, &mut vcx).is_empty());
    assert!(vcx.update(|_, cx| ws.read(cx).is_blank()));
}

#[gpui::test]
fn an_item_without_a_descriptor_is_skipped(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let mut saved = saved_with(&[(TestItem::KIND, json!("kept"))], 0);
    // An item tab saved by an item that cannot serialise: no kind, no state.
    saved.dock_area.center.children[0]
        .children
        .push(PanelState::new(ITEM_PANEL_NAME));
    let report = restore(&ws, &mut vcx, &saved);
    assert_eq!(report.restored_items, 1);
    assert_eq!(
        report.skipped_items,
        [SkippedItem {
            kind: String::new(),
            reason: SkipReason::NoDescriptor
        }]
    );
}

#[gpui::test]
fn restore_never_closes_what_the_user_already_opened(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open_split_workspace(&ws, &mut vcx);
    let saved = serialize(&ws, &mut vcx);
    let (fresh, mut fresh_vcx) = second_workspace(cx);
    open(&fresh, &mut fresh_vcx, "mine");
    let report = restore(&fresh, &mut fresh_vcx, &saved);
    assert!(report.centre_kept && !report.centre_restored);
    assert_eq!(shape(&fresh, &mut fresh_vcx), "[mine]*0");
}

/// An item with no serialised kind: it can be open but never comes back.
struct Ephemeral {
    focus: gpui::FocusHandle,
}

impl gpui::EventEmitter<crate::ItemEvent> for Ephemeral {}

impl gpui::Focusable for Ephemeral {
    fn focus_handle(&self, _: &gpui::App) -> gpui::FocusHandle {
        self.focus.clone()
    }
}

impl gpui::Render for Ephemeral {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        gpui::div()
    }
}

impl crate::Item for Ephemeral {
    fn tab_content(&self, _: &gpui::App) -> crate::TabContent {
        crate::TabContent::new("ephemeral")
    }
}

#[gpui::test]
fn items_that_cannot_serialise_are_not_saved(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open(&ws, &mut vcx, "a");
    let ephemeral = vcx.update(|window, cx| {
        let item = cx.new(|cx| Ephemeral {
            focus: cx.focus_handle(),
        });
        ws.update(cx, |ws, cx| {
            let id = ws.open_item(item, window, cx);
            ws.move_item_to_split(id, SplitDirection::Right, window, cx);
            id
        })
    });
    vcx.run_until_parked();
    assert_eq!(shape(&ws, &mut vcx), "([a]*0 [ephemeral]*0)");
    let pane = vcx.update(|_, cx| ws.read(cx).pane_group(cx).pane_for_item(ephemeral).cloned());
    assert_eq!(active_pane(&ws, &mut vcx).id(), pane.expect("pane").id());

    let saved = serialize(&ws, &mut vcx);
    assert_eq!(
        saved.active_pane, None,
        "the active pane holds nothing restorable"
    );
    let (fresh, mut fresh_vcx) = second_workspace(cx);
    let report = restore(&fresh, &mut fresh_vcx, &saved);
    assert_eq!(report.restored_items, 1);
    assert!(report.skipped_items.is_empty(), "never even written");
    assert_eq!(shape(&fresh, &mut fresh_vcx), "[a]*0");
}

fn add_dock_panels(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
) -> (Entity<TestPanel>, Entity<TestPanel>, Entity<TestPanel>) {
    let left_a = add_panel(ws, vcx, DockPosition::Left, "nav", |p| p.priority = 1);
    let left_b = add_panel(ws, vcx, DockPosition::Left, "search", |p| p.priority = 2);
    let bottom = add_panel(ws, vcx, DockPosition::Bottom, "logs", |_| {});
    (left_a, left_b, bottom)
}

#[gpui::test]
fn dock_size_visibility_displayed_tab_and_panel_state_come_back(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open(&ws, &mut vcx, "a");
    add_dock_panels(&ws, &mut vcx);
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.resize_dock(DockPosition::Left, Unscaled(333.), window, cx);
            ws.resize_dock(DockPosition::Bottom, Unscaled(222.), window, cx);
            ws.toggle_dock(DockPosition::Bottom, window, cx);
        })
    });
    // Display the second panel of the left dock.
    let search = vcx.update(|_, cx| {
        ws.read(cx)
            .panels(cx)
            .into_iter()
            .nth(1)
            .expect("two left panels first")
            .panel_id()
    });
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| ws.toggle_panel_by_id(search, window, cx));
    });
    vcx.run_until_parked();
    let left = dock(&ws, &mut vcx, DockPosition::Left);
    assert_eq!(left.active_panel(), Some(search));
    assert!(!dock(&ws, &mut vcx, DockPosition::Bottom).is_open());
    let saved = serialize(&ws, &mut vcx);

    let (fresh, mut fresh_vcx) = second_workspace(cx);
    let (nav, search_panel, logs) = add_dock_panels(&fresh, &mut fresh_vcx);
    let report = restore(&fresh, &mut fresh_vcx, &saved);
    assert_eq!(
        report.docks_restored,
        [DockPosition::Left, DockPosition::Bottom]
    );
    let left = dock(&fresh, &mut fresh_vcx, DockPosition::Left);
    assert_eq!(left.size(), Unscaled(333.));
    assert!(left.is_open());
    assert_eq!(
        left.active_panel(),
        Some(search_panel.entity_id()),
        "the second tab of the left dock is displayed again"
    );
    let bottom = dock(&fresh, &mut fresh_vcx, DockPosition::Bottom);
    assert_eq!(bottom.size(), Unscaled(222.));
    assert!(!bottom.is_open(), "closed docks stay closed");

    // Each panel got its own saved state back.
    fresh_vcx.update(|_, cx| {
        assert_eq!(nav.read(cx).restored, Some(json!({"title": "nav"})));
        assert_eq!(
            search_panel.read(cx).restored,
            Some(json!({"title": "search"}))
        );
        assert_eq!(logs.read(cx).restored, Some(json!({"title": "logs"})));
    });
}

#[gpui::test]
fn a_saved_dock_with_no_panel_yet_is_left_out(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    add_panel(&ws, &mut vcx, DockPosition::Right, "right", |_| {});
    let saved = serialize(&ws, &mut vcx);
    let (fresh, mut fresh_vcx) = second_workspace(cx);
    let report = restore(&fresh, &mut fresh_vcx, &saved);
    assert!(report.docks_restored.is_empty());
    assert!(
        vcx.update(|_, cx| fresh.read(cx).dock(DockPosition::Right, cx))
            .is_none()
    );
}

#[gpui::test]
fn a_dock_saved_on_a_big_display_is_clamped_to_this_window(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    add_panel(&ws, &mut vcx, DockPosition::Left, "nav", |_| {});
    let mut saved = serialize(&ws, &mut vcx);
    let docked = saved.dock_area.left_dock.take().expect("a left dock");
    saved.dock_area.left_dock = Some(DockState::new(
        docked.panel().clone(),
        docked.placement(),
        px(100_000.),
        true,
    ));
    let (fresh, mut fresh_vcx) = second_workspace(cx);
    add_panel(&fresh, &mut fresh_vcx, DockPosition::Left, "nav", |_| {});
    restore(&fresh, &mut fresh_vcx, &saved);
    let viewport = fresh_vcx.update(|window, _| f32::from(window.viewport_size().width));
    let size = dock(&fresh, &mut fresh_vcx, DockPosition::Left).size().0;
    assert!(
        size > 0. && size <= viewport * 0.8 + 0.5,
        "{size} of {viewport}"
    );
}

#[gpui::test]
fn dock_sizes_are_stored_unscaled_and_restore_at_another_zoom(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    add_panel(&ws, &mut vcx, DockPosition::Left, "nav", |_| {});
    vcx.update(|window, cx| {
        set_ui_scale(cx, UiScale::new(1.5));
        ws.update(cx, |ws, cx| {
            ws.resize_dock(DockPosition::Left, Unscaled(300.), window, cx)
        });
    });
    vcx.run_until_parked();
    let saved = serialize(&ws, &mut vcx);
    let stored = saved
        .dock_area
        .left_dock
        .as_ref()
        .expect("left dock")
        .size();
    assert!(
        (f32::from(stored) - 300.).abs() < 0.5,
        "unscaled, got {stored:?}"
    );

    let (fresh, mut fresh_vcx) = second_workspace(cx);
    add_panel(&fresh, &mut fresh_vcx, DockPosition::Left, "nav", |_| {});
    fresh_vcx.update(|_, cx| set_ui_scale(cx, UiScale::IDENTITY));
    restore(&fresh, &mut fresh_vcx, &saved);
    let size = fresh_vcx.update(|_, cx| fresh.read(cx).dock_size_unscaled(DockPosition::Left, cx));
    assert!((size.expect("dock") - 300.).abs() < 0.5, "{size:?}");
}
