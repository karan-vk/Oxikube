//! One view, two mounting modes: pinning the drawer as a tab keeps its state, and the tab moves
//! between panes.

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use oxikube_workspace::SplitDirection;

use super::fixture::{Detail, pod_ref, web_pod, web_replicaset};
use crate::detail::{DetailTab, DetailView, Mount, item_key};

#[gpui::test]
fn pinning_makes_the_same_view_a_workspace_tab_with_its_state(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod(), web_replicaset()]);
    let target = pod_ref("web-0");
    let view = d.open(&target);
    // State the user built up in the drawer.
    d.click("detail-tab-events");
    d.update(&view, |v, cx| {
        v.toggle_expanded(true, "note", cx);
        v.set_tab(DetailTab::Events, cx);
    });
    let id = view.entity_id();

    d.f.dispatcher.clear();
    d.click("detail-pin");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::ResourcePinDetail {
            target: target.clone()
        }]
    );

    // The tab is the same entity, mounted as a tab; the drawer let go of it.
    let workspace = d.workspace();
    let tabs: Vec<_> =
        d.f.vcx
            .update(|_, cx| workspace.read(cx).items_of_type::<DetailView>());
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].entity_id(), id, "promoted, not rebuilt");
    assert_eq!(d.read(&tabs[0], |v| v.mount()), Mount::Tab);
    assert_eq!(
        d.read(&tabs[0], |v| v.tab()),
        DetailTab::Events,
        "the active tab stays"
    );
    assert!(
        d.read(&tabs[0], |v| v.is_expanded(true, "note")),
        "so do expanded values"
    );
    assert!(d.drawer_view().is_none());
    let right_open = d.f.vcx.update(|_, cx| {
        workspace
            .read(cx)
            .dock(oxikube_workspace::DockPosition::Right, cx)
            .map(|dock| dock.is_open())
    });
    assert_eq!(right_open, Some(false), "the drawer closed");
    let active = d.f.vcx.update(|_, cx| {
        workspace
            .read(cx)
            .active_item(cx)
            .map(|item| item.item_id())
    });
    assert_eq!(active, Some(id), "the pinned tab is displayed");
    assert!(
        !d.shown("detail-pin"),
        "a tab has no pin or close button of the drawer's"
    );

    // Opening the object again shows the tab, not a second drawer.
    d.open(&target);
    assert!(d.drawer_view().is_none());
    let count =
        d.f.vcx
            .update(|_, cx| workspace.read(cx).items_of_type::<DetailView>().len());
    assert_eq!(count, 1);
}

#[gpui::test]
fn a_pinned_tab_survives_a_pane_move(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod(), web_replicaset()]);
    let target = pod_ref("web-0");
    // A first tab (the pods table) so the pane has something to split from.
    d.f.open_pods();
    let view = d.open(&target);
    d.update(&view, |v, cx| v.set_tab(DetailTab::Events, cx));
    d.f.dispatcher.clear();
    d.click("detail-pin");

    let workspace = d.workspace();
    let id = view.entity_id();
    let moved = d.f.vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.move_item_to_split(id, SplitDirection::Right, window, cx)
        })
    });
    assert!(moved.is_some(), "the tab moved to a pane of its own");
    d.settle();
    let (panes, in_new_pane, key) = d.f.vcx.update(|_, cx| {
        let ws = workspace.read(cx);
        let group = ws.pane_group(cx);
        let item = ws.item(id).expect("still open");
        (
            ws.panes(cx).len(),
            group.pane_for_item(id).map(|p| p.len()),
            item.item_key(cx).map(|k| k.to_string()),
        )
    });
    assert_eq!(panes, 2);
    assert_eq!(in_new_pane, Some(1));
    assert_eq!(key, Some(item_key(&target)));
    assert_eq!(
        d.read(&view, |v| v.tab()),
        DetailTab::Events,
        "state survives the move"
    );
    assert_eq!(d.read(&view, |v| v.mount()), Mount::Tab);
    assert!(
        d.read(&view, |v| v.model().is_some()),
        "and it still follows its object"
    );
}

#[gpui::test]
fn tab_switching_keeps_the_scroll_and_the_active_tab(cx: &mut TestAppContext) {
    let many = super::fixture::edited(web_pod(), |json| {
        let labels: serde_json::Map<String, serde_json::Value> = (0..300)
            .map(|i| (format!("label-{i:03}"), serde_json::json!(format!("v{i}"))))
            .collect();
        json["metadata"]["labels"] = serde_json::Value::Object(labels);
    });
    let mut d = Detail::new(cx, [many]);
    let view = d.open(&pod_ref("web-0"));
    d.draw();
    let list = d.read(&view, |v| v.overview_list().clone());
    list.scroll_by(gpui::px(900.));
    d.draw();
    let before = list.logical_scroll_top();
    assert!(before.item_ix > 10, "scrolled into the list: {before:?}");

    d.click("detail-tab-events");
    d.click("detail-tab-describe");
    d.click("detail-tab-overview");
    d.draw();
    let after = list.logical_scroll_top();
    assert_eq!(
        (after.item_ix, after.offset_in_item),
        (before.item_ix, before.offset_in_item),
        "the overview is where it was"
    );
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::Overview);
}
