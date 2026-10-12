//! DaemonSets in the views (E12-S07): the table's columns, the registry entry and the detail
//! panel, over testkit fakes.

use futures::executor::block_on;
use gpui::{Entity, TestAppContext};
use jiff::Timestamp;
use oxikube_app::ColumnId;
use oxikube_domain::Resource;
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use serde_json::json;

use crate::detail::tests::fixture::Detail;
use crate::kinds::{DetailRenderer, KindSpecs, WorkloadPanel};
use crate::table::ResourceTable;
use crate::table::tests::fixture::{Fixture, cluster};

fn kind(group: &str, kind: &str, plural: &str) -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new(group, "v1", kind),
        preferred: true,
        plural: plural.into(),
        singular: kind.to_lowercase(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch", "delete"]),
        namespaced: true,
    }
}

fn daemonsets() -> ResourceKind {
    kind("apps", "DaemonSet", "daemonsets")
}

/// `kube-system/agent`: wanted on 3 nodes (the `disk=ssd` ones), running on 3, ready on 2,
/// updated on 3, available on 2.
fn agent() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "apps/v1", "kind": "DaemonSet",
        "metadata": {"name": "agent", "namespace": "kube-system", "uid": "ds-uid",
                     "resourceVersion": "7", "creationTimestamp": "2026-01-01T00:00:00Z"},
        "spec": {
            "selector": {"matchLabels": {"app": "agent"}},
            "updateStrategy": {"type": "RollingUpdate", "rollingUpdate": {"maxUnavailable": 1}},
            "template": {
                "metadata": {"labels": {"app": "agent"}},
                "spec": {
                    "nodeSelector": {"disk": "ssd", "kubernetes.io/os": "linux"},
                    "containers": [{"name": "agent", "image": "reg/agent:1.4"}]
                }
            }
        },
        "status": {"desiredNumberScheduled": 3, "currentNumberScheduled": 3, "numberReady": 2,
                   "updatedNumberScheduled": 3, "numberAvailable": 2}
    }))
    .expect("a daemonset")
}

/// A pod of the agent DaemonSet on `node`.
fn agent_pod(name: &str, node: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": name, "namespace": "kube-system", "uid": format!("{name}-uid"),
                     "resourceVersion": "1", "creationTimestamp": "2026-01-01T00:00:00Z",
                     "labels": {"app": "agent"},
                     "ownerReferences": [{"apiVersion": "apps/v1", "kind": "DaemonSet",
                                          "name": "agent", "uid": "ds-uid", "controller": true}]},
        "spec": {"nodeName": node, "containers": [{"name": "agent", "image": "reg/agent:1.4"}]},
        "status": {"phase": "Running", "containerStatuses": [
            {"name": "agent", "ready": true, "restartCount": 0, "image": "reg/agent:1.4",
             "state": {"running": {"startedAt": "2026-01-01T00:00:00Z"}}}]}
    }))
    .expect("a pod")
}

fn agent_ref() -> ResourceRef {
    ResourceRef::namespaced(
        cluster(),
        Gvk::new("apps", "v1", "DaemonSet"),
        "kube-system",
        "agent",
    )
}

fn visible(f: &mut Fixture, table: &Entity<ResourceTable>) -> Vec<String> {
    f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            d.layout()
                .visible_ids()
                .iter()
                .map(ToString::to_string)
                .collect()
        })
    })
}

fn cell_text(f: &mut Fixture, table: &Entity<ResourceTable>, column: &str) -> String {
    f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let object = d.row(0).expect("a row");
            d.provider()
                .cell(object, &ColumnId::new(column), Timestamp::now())
                .display()
                .to_owned()
        })
    })
}

#[gpui::test]
fn the_daemonset_table_shows_the_columns_of_the_story_in_order(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    let ports = f.ports();
    ports.discovery.set_kinds([daemonsets()]);
    ports.resources.insert(agent());
    block_on(f.sessions.connect(&cluster())).expect("connect");
    f.vcx.run_until_parked();
    let table = f.open(daemonsets());
    let shown = visible(&mut f, &table);
    assert_eq!(
        shown,
        [
            "name",
            "namespace",
            "desired",
            "current",
            "ready",
            "up-to-date",
            "available",
            "node-selector",
            "age",
        ],
        "Desired, Current, Ready, Up-to-date, Available, Node Selector (wide columns stay hidden)"
    );
    for (column, text) in [
        ("desired", "3"),
        ("current", "3"),
        ("ready", "2"),
        ("up-to-date", "3"),
        ("available", "2"),
        ("node-selector", "disk=ssd,kubernetes.io/os=linux"),
    ] {
        assert_eq!(cell_text(&mut f, &table, column), text, "{column}");
    }
    assert_eq!(f.names(&table), ["agent"]);
}

#[gpui::test]
fn the_daemonset_has_a_panel_no_scale_and_keeps_its_sidebar_place(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    f.vcx.update(|_, cx| {
        let gvk = Gvk::new("apps", "v1", "DaemonSet");
        let meta = KindSpecs::kind_for(cx, &gvk);
        assert!(!meta.offers(CommandId::WORKLOAD_SCALE), "one pod per node");
        assert!(matches!(
            KindSpecs::detail_for(cx, &gvk),
            DetailRenderer::Panel(_)
        ));
        let tree = KindSpecs::registry(cx).sidebar_tree(None);
        let workloads = tree
            .sections
            .iter()
            .find(|s| s.section.id == "workloads")
            .expect("the Workloads section");
        let order: Vec<_> = workloads
            .entries
            .iter()
            .map(|e| &*e.meta.gvk.kind)
            .collect();
        assert_eq!(&order[..3], ["Pod", "Deployment", "DaemonSet"]);
    });
}

#[gpui::test]
fn the_panel_shows_counts_strategy_node_selector_and_the_pods_it_owns(cx: &mut TestAppContext) {
    let mut d = Detail::with_workloads(
        cx,
        [daemonsets(), kind("", "Pod", "pods")],
        [
            agent(),
            agent_pod("agent-aaaaa", "node-1"),
            agent_pod("agent-bbbbb", "node-2"),
        ],
    );
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    let view = d.open(&agent_ref());
    d.draw();
    let panel = d
        .read(&view, |v| v.kind_panel().cloned())
        .expect("the kind panel")
        .downcast::<WorkloadPanel>()
        .expect("a workload panel");
    d.settle();
    let model =
        d.f.vcx
            .update(|_, cx| panel.read(cx).model().cloned())
            .expect("a model");
    let stats: Vec<_> = model
        .stats
        .iter()
        .map(|s| (s.label, s.value.as_str()))
        .collect();
    assert_eq!(
        stats,
        [
            ("Desired", "3"),
            ("Current", "3"),
            ("Ready", "2"),
            ("Up-to-date", "3"),
            ("Available", "2")
        ]
    );
    assert_eq!(
        model.strategy.as_deref(),
        Some("RollingUpdate · Max unavailable 1")
    );
    assert_eq!(model.selector, "app=agent");
    assert!(
        model.extras.contains(&(
            "Node selector",
            "disk=ssd, kubernetes.io/os=linux".to_owned()
        )),
        "{:?}",
        model.extras
    );
    let pods = d.f.vcx.update(|_, cx| panel.read(cx).pod_rows().clone());
    let names: Vec<_> = pods
        .rows
        .iter()
        .map(|r| r.target.name.to_string())
        .collect();
    assert_eq!(names, ["agent-aaaaa", "agent-bbbbb"]);
    for part in [
        "workload-panel",
        "workload-stats",
        "workload-strategy",
        "workload-pods",
    ] {
        assert!(d.shown(part), "{part} is drawn");
    }
    assert!(!d.shown("workload-scale"), "a DaemonSet cannot be scaled");
}

#[test]
fn an_unconstrained_daemonset_says_every_node() {
    let res = Resource::from_json(json!({
        "apiVersion": "apps/v1", "kind": "DaemonSet",
        "metadata": {"name": "agent", "namespace": "kube-system", "uid": "ds-uid"},
        "spec": {"template": {"spec": {"containers": [{"name": "a", "image": "i"}]}}}
    }))
    .expect("a daemonset");
    let model = crate::kinds::workloads::panel::PanelModel::build(&res).expect("a model");
    assert!(
        model
            .extras
            .contains(&("Node selector", "none (every node)".to_owned()))
    );
}
