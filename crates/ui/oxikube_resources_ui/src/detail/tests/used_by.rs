//! The Used by tab (E12-S18): the workloads whose Pods name a ConfigMap, Secret, ServiceAccount,
//! claim or PriorityClass, listed in the detail, opened with a click, over a Pods subscription the
//! tab holds only while it is up.

use std::time::Duration;

use gpui::TestAppContext;
use oxikube_app::{UsedBy, WorkloadUse};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use serde_json::{Value, json};

use super::fixture::{Detail, edited, pod_ref, web_pod};
use crate::detail::DetailTab;
use crate::table::tests::fixture::cluster;
use oxikube_testkit::{deployment, replicaset};

fn object_ref(kind: &str, name: &str) -> ResourceRef {
    let gvk = match kind {
        "Deployment" | "ReplicaSet" => Gvk::new("apps", "v1", kind),
        _ => Gvk::new("", "v1", kind),
    };
    ResourceRef::namespaced(cluster(), gvk, "shop", name)
}

fn config_map(name: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": name, "namespace": "shop", "resourceVersion": "3"},
        "data": {"k": "v"},
    }))
    .unwrap()
}

/// A Pod `shop/<name>` whose `spec` gets `spec`, owned by the ReplicaSet `owner` when there is one.
fn pod(name: &str, owner: Option<&str>, spec: Value) -> Resource {
    let mut json = json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": name, "namespace": "shop", "uid": format!("uid-{name}"),
                     "resourceVersion": "1"},
        "spec": spec,
        "status": {"phase": "Running"},
    });
    if let Some(owner) = owner {
        json["metadata"]["ownerReferences"] = json!([{
            "apiVersion": "apps/v1", "kind": "ReplicaSet", "name": owner,
            "uid": format!("uid-{owner}"), "controller": true,
        }]);
    }
    Resource::from_json(json).unwrap()
}

fn mounts(cm: &str) -> Value {
    json!({"volumes": [{"name": "v", "configMap": {"name": cm}}]})
}

/// The ReplicaSet `web-5d`, owned by the Deployment `web`, and the Deployment.
fn web_deployment() -> [Resource; 2] {
    let rs = edited(
        replicaset().namespace("shop").name("web-5d").build(),
        |json| {
            json["metadata"]["ownerReferences"] = json!([{
                "apiVersion": "apps/v1", "kind": "Deployment", "name": "web",
                "uid": "uid-web", "controller": true,
            }]);
        },
    );
    [rs, deployment().namespace("shop").name("web").build()]
}

/// Lets the Pods feed land and the tab's polls (every 200 ms while it is reading) pick it up.
fn settle_used_by(d: &mut Detail) {
    for _ in 0..4 {
        d.f.vcx.executor().advance_clock(Duration::from_millis(300));
        d.settle();
    }
}

fn workloads(d: &mut Detail, view: &gpui::Entity<crate::detail::DetailView>) -> Vec<WorkloadUse> {
    match d.read(view, |v| v.used_by_answer().cloned()) {
        Some(UsedBy::Known { workloads, .. }) => workloads,
        other => panic!("expected an answer, got {other:?}"),
    }
}

#[gpui::test]
fn only_the_referenced_kinds_have_a_used_by_tab(cx: &mut TestAppContext) {
    for kind in [
        "ServiceAccount",
        "ConfigMap",
        "Secret",
        "PersistentVolumeClaim",
    ] {
        let gvk = Gvk::new("", "v1", kind);
        assert!(
            DetailTab::for_kind(&gvk).contains(&DetailTab::UsedBy),
            "{kind}"
        );
    }
    let class = Gvk::new("scheduling.k8s.io", "v1", "PriorityClass");
    assert!(DetailTab::for_kind(&class).contains(&DetailTab::UsedBy));
    assert!(!DetailTab::for_kind(&Gvk::new("", "v1", "Pod")).contains(&DetailTab::UsedBy));
    assert!(
        !DetailTab::for_kind(&Gvk::new("apps", "v1", "Deployment")).contains(&DetailTab::UsedBy)
    );
    assert_eq!(DetailTab::UsedBy.title(), "Used by");

    let mut d = Detail::new(cx, [config_map("cfg"), web_pod()]);
    d.open(&pod_ref("web-0"));
    assert!(!d.shown("detail-tab-used-by"), "a pod has none");
    d.open(&object_ref("ConfigMap", "cfg"));
    assert!(d.shown("detail-tab-used-by"));
}

#[gpui::test]
fn a_config_maps_used_by_tab_lists_the_deployment_and_a_click_opens_it(cx: &mut TestAppContext) {
    let [rs, web] = web_deployment();
    let mut d = Detail::new(
        cx,
        [
            config_map("cfg"),
            rs,
            web,
            pod("web-5d-a", Some("web-5d"), mounts("cfg")),
            pod("web-5d-b", Some("web-5d"), mounts("cfg")),
            pod("other", Some("web-5d"), mounts("not-cfg")),
        ],
    );
    let view = d.open(&object_ref("ConfigMap", "cfg"));
    d.click("detail-tab-used-by");
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::UsedBy);
    settle_used_by(&mut d);

    assert_eq!(
        workloads(&mut d, &view),
        [WorkloadUse {
            target: object_ref("Deployment", "web"),
            pods: 2
        }],
        "two pods of one replica set are one Deployment"
    );
    assert!(d.shown("detail-used-by-summary"));
    assert!(d.shown("used-by-row-0"));
    assert!(!d.shown("used-by-row-1"));

    d.f.dispatcher.clear();
    d.click("used-by-row-0");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::ResourceOpen {
            target: object_ref("Deployment", "web")
        }],
        "navigation is resource::Open of the workload"
    );
    // The drawer now shows the Deployment.
    d.settle();
    let shown = d.drawer_view().expect("the drawer");
    assert_eq!(
        d.read(&shown, |v| v.target().clone()),
        object_ref("Deployment", "web")
    );
}

#[gpui::test]
fn an_unowned_pod_lists_itself_and_an_unused_object_says_so(cx: &mut TestAppContext) {
    let mut d = Detail::new(
        cx,
        [
            config_map("cfg"),
            config_map("spare"),
            pod("lone", None, mounts("cfg")),
        ],
    );
    let view = d.open(&object_ref("ConfigMap", "cfg"));
    d.update(&view, |v, cx| v.set_tab(DetailTab::UsedBy, cx));
    settle_used_by(&mut d);
    assert_eq!(
        workloads(&mut d, &view),
        [WorkloadUse {
            target: object_ref("Pod", "lone"),
            pods: 1
        }]
    );

    let spare = d.open(&object_ref("ConfigMap", "spare"));
    d.update(&spare, |v, cx| v.set_tab(DetailTab::UsedBy, cx));
    settle_used_by(&mut d);
    assert!(workloads(&mut d, &spare).is_empty());
    assert!(
        d.shown("detail-used-by-summary"),
        "it says no running pod uses it"
    );
    assert!(!d.shown("used-by-row-0"));
}

#[gpui::test]
fn the_tab_follows_the_pods_while_it_is_up(cx: &mut TestAppContext) {
    use oxikube_ports::{Delta, DeltaBatch};
    use oxikube_testkit::Timeline;
    let mut d = Detail::new(cx, [config_map("cfg")]);
    // Pods: one at the start; a second 10 seconds later; the first leaves after 20.
    let first = pod("lone-a", None, mounts("cfg"));
    let second = pod("lone-b", None, mounts("cfg"));
    d.f.ports().resources.script().watch.push_ok(
        Timeline::immediate([DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
            config_map("cfg"),
        ])])])
        .keep_open(),
    );
    d.f.ports().resources.script().watch.push_ok(
        Timeline::immediate([DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
            first.clone(),
        ])])])
        .ok_at(
            Duration::from_secs(10),
            DeltaBatch::from_deltas(vec![Delta::Applied(second)]),
        )
        .ok_at(
            Duration::from_secs(20),
            DeltaBatch::from_deltas(vec![Delta::Deleted(first)]),
        )
        .keep_open(),
    );
    let view = d.open(&object_ref("ConfigMap", "cfg"));
    d.update(&view, |v, cx| v.set_tab(DetailTab::UsedBy, cx));
    settle_used_by(&mut d);
    assert_eq!(workloads(&mut d, &view).len(), 1);

    d.f.ports()
        .resources
        .clock()
        .advance(Duration::from_secs(10));
    d.settle();
    settle_used_by(&mut d);
    let names: Vec<String> = workloads(&mut d, &view)
        .iter()
        .map(|w| w.target.name.to_string())
        .collect();
    assert_eq!(names, ["lone-a", "lone-b"]);

    d.f.ports()
        .resources
        .clock()
        .advance(Duration::from_secs(10));
    d.settle();
    settle_used_by(&mut d);
    let names: Vec<String> = workloads(&mut d, &view)
        .iter()
        .map(|w| w.target.name.to_string())
        .collect();
    assert_eq!(names, ["lone-b"], "a deleted pod is not a user any more");
}

#[gpui::test]
fn the_pods_feed_is_held_only_while_the_tab_is_up(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [config_map("cfg"), pod("lone", None, mounts("cfg"))]);
    let live = |d: &mut Detail| d.f.ports().resources.live_watches();
    let view = d.open(&object_ref("ConfigMap", "cfg"));
    d.settle();
    let before = live(&mut d);
    assert!(
        !d.read(&view, |v| v.holds_reference_feed()),
        "the Overview needs no Pods"
    );

    d.update(&view, |v, cx| v.set_tab(DetailTab::UsedBy, cx));
    settle_used_by(&mut d);
    assert!(d.read(&view, |v| v.holds_reference_feed()));
    assert_eq!(
        live(&mut d),
        before + 1,
        "the Pods feed started with the tab"
    );

    // Leaving the tab releases the lease; the store stops the feed after its grace period.
    d.update(&view, |v, cx| v.set_tab(DetailTab::Overview, cx));
    assert!(!d.read(&view, |v| v.holds_reference_feed()));
    assert!(
        d.read(&view, |v| v.used_by_answer().is_none()),
        "nothing is kept"
    );
    d.f.clock.advance(Duration::from_secs(60));
    d.settle();
    assert_eq!(
        live(&mut d),
        before,
        "the Pods feed ref is released with the tab"
    );

    // Coming back starts it again.
    d.update(&view, |v, cx| v.set_tab(DetailTab::UsedBy, cx));
    settle_used_by(&mut d);
    assert_eq!(live(&mut d), before + 1);
    assert_eq!(workloads(&mut d, &view).len(), 1);
}

#[gpui::test]
fn two_tabs_share_one_pods_feed_and_the_last_to_close_releases_it(cx: &mut TestAppContext) {
    let mut d = Detail::new(
        cx,
        [
            config_map("cfg"),
            config_map("cfg2"),
            pod("lone", None, mounts("cfg")),
        ],
    );
    let first = d.open(&object_ref("ConfigMap", "cfg"));
    d.update(&first, |v, cx| v.set_tab(DetailTab::UsedBy, cx));
    settle_used_by(&mut d);
    let pod_watches = |d: &mut Detail| {
        d.f.ports()
            .resources
            .recorded_calls()
            .iter()
            .filter(
                |c| matches!(c, oxikube_testkit::ResourceCall::Watch { kind, .. } if kind.is_pod()),
            )
            .count()
    };
    assert_eq!(pod_watches(&mut d), 1);

    let second = d.open(&object_ref("ConfigMap", "cfg2"));
    d.update(&second, |v, cx| v.set_tab(DetailTab::UsedBy, cx));
    settle_used_by(&mut d);
    assert_eq!(
        pod_watches(&mut d),
        1,
        "the second tab shares the Pods feed"
    );
    assert!(d.read(&first, |v| v.holds_reference_feed()));
    assert!(d.read(&second, |v| v.holds_reference_feed()));

    // One closes: the other still holds the feed, so the Pods are still watched.
    let pods_live = |d: &mut Detail| d.f.ports().resources.live_watches();
    let both = pods_live(&mut d);
    d.update(&first, |v, cx| v.set_tab(DetailTab::Overview, cx));
    d.f.clock.advance(Duration::from_secs(60));
    d.settle();
    assert_eq!(pods_live(&mut d), both, "still held by the second tab");
    assert!(
        workloads(&mut d, &second).is_empty(),
        "cfg2 is used by nobody, and it knows"
    );

    // The last one closes: the feed goes.
    d.update(&second, |v, cx| v.set_tab(DetailTab::Overview, cx));
    d.f.clock.advance(Duration::from_secs(60));
    d.settle();
    assert_eq!(pods_live(&mut d), both - 1, "the Pods feed ref is released");
}

#[gpui::test]
fn the_tab_says_why_it_cannot_answer(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [config_map("cfg")]);
    let view = d.open(&object_ref("ConfigMap", "cfg"));
    d.update(&view, |v, cx| v.set_tab(DetailTab::UsedBy, cx));
    for (answer, selector) in [
        (UsedBy::Warming, "detail-used-by-warming"),
        (
            UsedBy::Unavailable("pods is forbidden".into()),
            "detail-used-by-unavailable",
        ),
        (UsedBy::OutsideScope, "detail-used-by-outside"),
    ] {
        d.update(&view, |v, cx| v.set_used_by(answer, cx));
        assert!(d.shown(selector), "{selector}");
    }
    let partial = UsedBy::Known {
        workloads: vec![],
        pods: 0,
        partial: true,
    };
    d.update(&view, |v, cx| v.set_used_by(partial, cx));
    assert!(d.shown("detail-used-by-partial"));
    assert!(d.shown("detail-used-by-basis"));
}

#[gpui::test]
fn show_used_by_opens_the_detail_on_its_tab_through_the_bus_request(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [config_map("cfg"), web_pod()]);
    let target = object_ref("ConfigMap", "cfg");
    d.f.vcx.update(|_, cx| {
        use oxikube_workspace::CommandDispatcher as _;
        d.f.dispatcher.dispatch(
            Command::ResourceShowUsedBy {
                target: target.clone(),
            },
            cx,
        );
    });
    d.settle();
    let view = d.drawer_view().expect("the drawer opened");
    assert_eq!(d.read(&view, |v| v.target().clone()), target);
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::UsedBy);

    // A kind without the tab keeps the tab it has: nothing opens for a pod.
    let pod_target = pod_ref("web-0");
    d.f.vcx.update(|_, cx| {
        use oxikube_workspace::CommandDispatcher as _;
        d.f.dispatcher.dispatch(
            Command::ResourceShowUsedBy {
                target: pod_target.clone(),
            },
            cx,
        );
    });
    d.settle();
    let view = d.drawer_view().expect("the drawer");
    assert_eq!(
        d.read(&view, |v| v.target().clone()),
        target,
        "the drawer did not move"
    );
}
