//! Opening a detail: a pod and a custom resource show header, labels, owner link and conditions;
//! an owner link opens the owner; the tabs and the events.

use std::sync::Arc;
use std::time::Duration;

use gpui::TestAppContext;
use oxikube_app::columns::Tone;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_ports::{Delta, DeltaBatch, TableBatch, TableColumn, TableRow, TableSource};
use oxikube_ports::{DescribeOutput, DescribeSource};
use oxikube_testkit::Timeline;
use serde_json::json;

use super::fixture::{Detail, edited, pod_ref, web_pod, web_replicaset};
use crate::detail::model::{Row, Section};
use crate::detail::{DetailState, DetailTab, Mount};
use crate::table::tests::fixture::cluster;

#[gpui::test]
fn a_pod_opens_in_the_drawer_with_header_labels_owner_and_conditions(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod(), web_replicaset()]);
    let view = d.open(&pod_ref("web-0"));

    // The drawer is the right dock of the cluster tab, showing this view.
    assert_eq!(d.drawer_view().as_ref(), Some(&view));
    let workspace = d.workspace();
    let open = d.f.vcx.update(|_, cx| {
        workspace
            .read(cx)
            .dock(oxikube_workspace::DockPosition::Right, cx)
            .map(|dock| dock.is_open())
    });
    assert_eq!(open, Some(true), "opening a detail opens the right dock");

    assert_eq!(d.read(&view, |v| v.state().clone()), DetailState::Live);
    assert_eq!(d.read(&view, |v| v.mount()), Mount::Drawer);
    let model = d.read(&view, |v| v.model().cloned()).expect("a model");
    assert_eq!(&*model.header.kind, "Pod");
    assert_eq!(&*model.header.name, "web-0");
    assert_eq!(model.header.namespace.as_deref(), Some("shop"));
    let chip = model.header.status.clone().expect("a status chip");
    assert_eq!((chip.text.as_str(), chip.tone), ("Running", Tone::Ok));
    assert_eq!(
        model
            .labels
            .iter()
            .map(|l| l.copy_text())
            .collect::<Vec<_>>(),
        ["app=web", "tier=frontend"]
    );
    assert_eq!(model.finalizers, ["example.com/cleanup"]);
    let kinds: Vec<&str> = model.conditions.iter().map(|c| c.kind.as_str()).collect();
    assert_eq!(kinds, ["Ready", "PodScheduled"]);

    // What is on screen.
    for selector in [
        "detail-view",
        "detail-kind",
        "detail-name",
        "detail-namespace",
        "detail-age",
        "detail-status",
        "detail-pin",
        "detail-close",
        "detail-tab-overview",
        "detail-tab-yaml",
        "detail-tab-describe",
        "detail-tab-events",
        "detail-label-0",
        "detail-label-1",
        "detail-annotation-0",
        "detail-owner-0",
        "detail-finalizer-0",
        "detail-condition-head",
        "detail-condition-0",
        "detail-condition-1",
    ] {
        assert!(d.shown(selector), "{selector} is on screen");
    }
    assert!(!d.shown("detail-banner"), "a live object has no banner");
}

#[gpui::test]
fn the_owner_link_opens_the_owners_detail(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod(), web_replicaset()]);
    let view = d.open(&pod_ref("web-0"));
    let owner = d
        .read(&view, |v| v.model().and_then(|m| m.owners.first().cloned()))
        .expect("an owner");
    let target = d
        .read(&view, |v| v.owner_target(&owner))
        .expect("discovery resolved the scope");
    assert_eq!(
        target,
        ResourceRef::namespaced(
            cluster(),
            Gvk::new("apps", "v1", "ReplicaSet"),
            "shop",
            "web-5d"
        ),
        "a namespaced owner is in the object's namespace, found by type, not by text"
    );

    d.f.dispatcher.clear();
    d.click("detail-owner-0");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::ResourceOpen {
            target: target.clone()
        }],
        "the click is a command"
    );
    let shown = d.drawer_view().expect("the drawer shows a detail");
    assert_eq!(d.read(&shown, |v| v.target().clone()), target);
    assert_ne!(shown, view, "the owner has its own view");
    let owner_model = d
        .read(&shown, |v| v.model().cloned())
        .expect("owner loaded");
    assert_eq!(&*owner_model.header.kind, "ReplicaSet");
}

#[gpui::test]
fn an_owner_of_a_cluster_scoped_kind_links_without_a_namespace(cx: &mut TestAppContext) {
    let node = edited(oxikube_testkit::node().name("node-1").build(), |_| {});
    let lease = edited(
        oxikube_testkit::resource("coordination.k8s.io/v1", "Lease")
            .namespace("kube-node-lease")
            .name("node-1")
            .build(),
        |json| {
            json["metadata"]["ownerReferences"] = json!([{
                "apiVersion": "v1", "kind": "Node", "name": "node-1", "uid": "n"
            }]);
        },
    );
    let mut d = Detail::new(cx, [node, lease]);
    // The kinds: Lease is namespaced, Node is not.
    d.f.ports()
        .discovery
        .set_kinds(
            super::fixture::kinds()
                .into_iter()
                .chain([super::fixture::kind(
                    "coordination.k8s.io",
                    "v1",
                    "Lease",
                    "leases",
                    true,
                )]),
        );
    let target = ResourceRef::namespaced(
        cluster(),
        Gvk::new("coordination.k8s.io", "v1", "Lease"),
        "kube-node-lease",
        "node-1",
    );
    let view = d.open(&target);
    let owner = d
        .read(&view, |v| v.model().and_then(|m| m.owners.first().cloned()))
        .expect("an owner");
    let owner_target = d.read(&view, |v| v.owner_target(&owner)).expect("resolved");
    assert_eq!(owner_target.namespace, None);
    assert_eq!(owner_target.gvk, Gvk::new("", "v1", "Node"));
}

#[gpui::test]
fn an_owner_of_an_unknown_kind_is_shown_but_does_not_open(cx: &mut TestAppContext) {
    let orphan = edited(web_pod(), |json| {
        json["metadata"]["ownerReferences"] = json!([{
            "apiVersion": "mystery.io/v1", "kind": "Ghost", "name": "boo", "uid": "g"
        }]);
    });
    let mut d = Detail::new(cx, [orphan]);
    let view = d.open(&pod_ref("web-0"));
    assert!(d.shown("detail-owner-0"));
    d.f.dispatcher.clear();
    d.click("detail-owner-0");
    assert!(d.f.dispatcher.sent().is_empty(), "nothing to open");
    let _ = view;
}

/// A Widget custom resource: served through the Table feed (server columns), so the feed has
/// metadata only and the detail reads the object once for its status.
fn widget() -> oxikube_domain::Resource {
    edited(
        oxikube_testkit::resource("example.com/v1", "Widget")
            .namespace("shop")
            .name("gizmo")
            .label("team", "blue")
            .field("spec", json!({"size": "large"}))
            .field(
                "status",
                json!({
                    "phase": "Active",
                    "observedGeneration": 4,
                    "conditions": [
                        {"type": "Ready", "status": "True", "reason": "AllGood",
                         "lastTransitionTime": "2026-01-01T00:00:00Z"}
                    ]
                }),
            )
            .created("2026-01-01T00:00:00Z")
            .build(),
        |json| {
            json["metadata"]["resourceVersion"] = json!("11");
        },
    )
}

fn script_widget_table(d: &mut Detail, widget: &oxikube_domain::Resource) {
    let columns: Arc<[TableColumn]> = Arc::from(vec![
        TableColumn {
            name: "Name".into(),
            column_type: "string".into(),
            ..TableColumn::default()
        },
        TableColumn {
            name: "Status".into(),
            column_type: "string".into(),
            ..TableColumn::default()
        },
    ]);
    let row = TableRow {
        cells: vec![json!("gizmo"), json!("Active")],
        meta: Some(widget.meta.clone()),
        object: None,
    };
    d.f.ports().tables.script().table_feed.push_ok(
        Timeline::immediate([TableBatch {
            columns: Some(columns),
            rows: DeltaBatch::from_deltas(vec![Delta::Restarted(vec![row])]),
            source: TableSource::Server,
        }])
        .keep_open(),
    );
}

#[gpui::test]
fn a_custom_resource_opens_through_the_table_feed_and_one_full_read(cx: &mut TestAppContext) {
    let widget = widget();
    let mut d = Detail::new(cx, [widget.clone()]);
    script_widget_table(&mut d, &widget);
    let target = ResourceRef::namespaced(
        cluster(),
        Gvk::new("example.com", "v1", "Widget"),
        "shop",
        "gizmo",
    );
    let view = d.open(&target);
    d.settle();

    let model = d.read(&view, |v| v.model().cloned()).expect("a model");
    assert_eq!(&*model.header.kind, "Widget");
    assert_eq!(model.header.namespace.as_deref(), Some("shop"));
    assert_eq!(
        model.header.status.as_ref().map(|c| c.text.as_str()),
        Some("Active"),
        "the chip is the server's Status column"
    );
    assert_eq!(
        model.labels[0].copy_text(),
        "team=blue",
        "labels from the row's metadata"
    );
    assert!(model.complete, "the full object was read");
    assert_eq!(model.conditions.len(), 1);
    assert_eq!(model.conditions[0].reason, "AllGood");
    let keys: Vec<&str> = model.status.lines.iter().map(|l| l.key.as_str()).collect();
    assert_eq!(keys, ["phase", "observedGeneration"]);
    assert!(d.shown("detail-condition-0"));
    assert!(d.shown("detail-status-0"));
}

#[gpui::test]
fn a_full_pod_needs_no_extra_read(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    d.open(&pod_ref("web-0"));
    let gets =
        d.f.ports()
            .resources
            .recorded_calls()
            .into_iter()
            .filter(|call| matches!(call, oxikube_testkit::ResourceCall::Get { .. }))
            .count();
    assert_eq!(gets, 0, "the reflector feed carries the whole object");
}

#[gpui::test]
fn opening_a_detail_does_not_block_and_shows_a_skeleton_first(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    let views = d.f.views.clone();
    let target = pod_ref("web-0");
    // No run_until_parked: the view exists, the store has not answered yet.
    let view =
        d.f.vcx
            .update(|window, cx| views.update(cx, |v, cx| v.open_detail(&target, window, cx)))
            .expect("a view");
    assert_eq!(d.read(&view, |v| v.state().clone()), DetailState::Loading);
    assert!(d.read(&view, |v| v.model().is_none()));
    assert!(
        d.shown("detail-skeleton"),
        "a skeleton stands in for the content"
    );
    d.settle();
    assert!(!d.shown("detail-skeleton"));
    assert!(d.shown("detail-label-0"));
}

#[gpui::test]
fn the_yaml_and_describe_tabs_show_the_object_and_events_follow_it(cx: &mut TestAppContext) {
    let warning = edited(
        oxikube_testkit::resource("v1", "Event")
            .namespace("shop")
            .name("web-0.backoff")
            .field("type", json!("Warning"))
            .field("reason", json!("BackOff"))
            .field("message", json!("Back-off restarting failed container"))
            .field("count", json!(3))
            .field("lastTimestamp", json!("2026-01-01T00:30:00Z"))
            .field(
                "involvedObject",
                json!({"apiVersion": "v1", "kind": "Pod", "name": "web-0",
                       "namespace": "shop", "uid": "u-web-0"}),
            )
            .build(),
        |_| {},
    );
    let elsewhere = edited(
        oxikube_testkit::resource("v1", "Event")
            .namespace("shop")
            .name("web-1.started")
            .field("type", json!("Normal"))
            .field("reason", json!("Started"))
            .field("message", json!("started web-1"))
            .field("lastTimestamp", json!("2026-01-01T00:31:00Z"))
            .field(
                "involvedObject",
                json!({"apiVersion": "v1", "kind": "Pod", "name": "web-1", "namespace": "shop"}),
            )
            .build(),
        |_| {},
    );
    let mut d = Detail::new(cx, [web_pod(), warning, elsewhere]);
    let view = d.open(&pod_ref("web-0"));
    let event_watches = |d: &mut Detail| {
        d.f.ports()
            .resources
            .recorded_calls()
            .into_iter()
            .filter(|call| {
                matches!(call, oxikube_testkit::ResourceCall::Watch { kind, .. } if &*kind.kind == "Event")
            })
            .count()
    };
    assert_eq!(
        event_watches(&mut d),
        0,
        "the Overview alone starts no events watch"
    );

    d.click("detail-tab-yaml");
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::Yaml);
    assert!(d.shown("detail-yaml"));
    d.f.ports()
        .describe
        .script()
        .describe
        .push_ok(DescribeOutput {
            text: "Name: web-0".into(),
            source: DescribeSource::Native,
        });
    d.click("detail-tab-describe");
    assert!(d.shown("detail-describe"));

    d.click("detail-tab-events");
    d.settle();
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::Events);
    assert_eq!(
        event_watches(&mut d),
        1,
        "the feed starts when the tab is first shown"
    );
    let rows = d.read(&view, |v| v.event_rows().to_vec());
    assert_eq!(rows.len(), 1, "only the events about this pod");
    assert_eq!(&*rows[0].reason, "BackOff");
    assert_eq!(rows[0].count, 3);
    assert!(d.shown("detail-event-0"));
    assert!(!d.shown("detail-event-1"));
    assert!(
        !d.shown("detail-condition-0"),
        "the Overview is not drawn behind it"
    );

    // Back and forth does not start a second watch.
    d.click("detail-tab-overview");
    d.click("detail-tab-events");
    assert_eq!(event_watches(&mut d), 1);
}

#[gpui::test]
fn the_events_tab_says_when_there_are_none(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    d.open(&pod_ref("web-0"));
    d.click("detail-tab-events");
    d.settle();
    assert!(d.shown("detail-events-empty"));
}

#[gpui::test]
fn a_live_update_changes_the_status_in_place(cx: &mut TestAppContext) {
    let pod = web_pod();
    let mut d = Detail::new(cx, [pod.clone()]);
    // The feed: the pod, then a new version a second later.
    let updated = edited(pod.clone(), |json| {
        json["metadata"]["resourceVersion"] = json!("8");
        json["metadata"]["labels"]["app"] = json!("web-v2");
    });
    d.f.ports().resources.script().watch.push_ok(
        Timeline::immediate([DeltaBatch::from_deltas(vec![Delta::Restarted(vec![pod])])])
            .ok_at(
                Duration::from_secs(1),
                DeltaBatch::from_deltas(vec![Delta::Applied(updated)]),
            )
            .keep_open(),
    );
    let view = d.open(&pod_ref("web-0"));
    assert_eq!(
        d.read(&view, |v| v.model().map(|m| m.labels[0].copy_text())),
        Some("app=web".to_owned())
    );
    d.f.ports()
        .resources
        .clock()
        .advance(Duration::from_secs(1));
    d.settle();
    assert_eq!(
        d.read(&view, |v| v.model().map(|m| m.labels[0].copy_text())),
        Some("app=web-v2".to_owned()),
        "the drawer re-renders on change"
    );
    assert!(d.read(&view, |v| {
        v.rows().contains(&Row::Section(Section::Labels, 2))
    }));
}

/// The namespace argument of every `Pod` watch the cluster saw, in order.
fn pod_watches(d: &mut Detail) -> Vec<Option<String>> {
    d.f.ports()
        .resources
        .recorded_calls()
        .into_iter()
        .filter_map(|call| match call {
            oxikube_testkit::ResourceCall::Watch {
                kind, namespace, ..
            } if &*kind.kind == "Pod" => Some(namespace),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn a_detail_alone_watches_one_namespace_never_the_whole_cluster(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    d.open(&pod_ref("web-0"));
    assert_eq!(
        pod_watches(&mut d),
        [Some("shop".to_owned())],
        "one object must not start a cluster-wide watch of every pod"
    );
}

#[gpui::test]
fn a_detail_shares_the_feed_the_table_already_holds(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    // The pods table, with every namespace selected: one cluster-wide feed.
    d.f.open_pods();
    assert_eq!(pod_watches(&mut d), [None]);
    let view = d.open(&pod_ref("web-0"));
    assert_eq!(
        pod_watches(&mut d),
        [None],
        "the detail rides the table's feed: no second watch"
    );
    assert!(
        d.read(&view, |v| v.model().is_some()),
        "and it still found the pod"
    );
}

#[gpui::test]
fn a_failed_scope_lookup_is_asked_again_not_remembered(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod(), web_replicaset()]);
    d.f.ports()
        .discovery
        .script()
        .resolve
        .push_err(oxikube_domain::OxiError::network("api server unreachable"));
    let view = d.open(&pod_ref("web-0"));
    let owner = d
        .read(&view, |v| v.model().and_then(|m| m.owners.first().cloned()))
        .expect("an owner");
    assert_eq!(
        d.read(&view, |v| v.owner_target(&owner)),
        None,
        "the lookup failed, so the link is not open yet"
    );

    // The cluster is back: the next rebuild asks again.
    d.update(&view, |v, cx| v.rebuild(cx));
    assert!(
        d.read(&view, |v| v.owner_target(&owner)).is_some(),
        "a transient failure is not cached as 'kind not served'"
    );
}
