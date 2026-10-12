//! The node detail panel: allocation bars from the pods on the node, taints, images, the pods
//! list, the live index and its field-selector fallback, and the cordon button.

use std::time::Duration;

use gpui::TestAppContext;
use oxikube_domain::OxiError;
use oxikube_domain::command::Command;
use oxikube_ports::{Delta, DeltaBatch, ListPage};
use oxikube_testkit::{ResourceCall, Timeline, node, pod};

use super::{cpu_requests, node_detail, node_ref, panel_of, pod_on, tick, worker};
use oxikube_app::allocation::AllocationKind;

/// Worker-1 with two scheduled pods, one on another node and one that finished.
fn busy_cluster() -> Vec<oxikube_domain::Resource> {
    vec![
        node()
            .name("worker-1")
            .taint("dedicated", "gpu", "NoSchedule")
            .image("docker.io/library/nginx:1.29", 190 << 20)
            .image("registry.k8s.io/pause:3.10", 700 << 10)
            .build(),
        pod_on("p1", "worker-1", ("250m", "256Mi"), ("500m", "512Mi")),
        pod_on("p2", "worker-1", ("750m", "1Gi"), ("1", "2Gi")),
        pod_on("elsewhere", "worker-2", ("3", "8Gi"), ("3", "8Gi")),
        pod()
            .namespace("x")
            .name("done")
            .node("worker-1")
            .succeeded()
            .build(),
    ]
}

#[gpui::test]
fn the_panel_draws_allocation_taints_images_and_pods(cx: &mut TestAppContext) {
    let mut d = node_detail(cx, busy_cluster());
    let view = d.open(&node_ref("worker-1"));
    let panel = panel_of(&mut d, &view);
    tick(&mut d);

    for selector in [
        "node-panel",
        "node-conditions",
        "node-condition-Ready",
        "node-condition-MemoryPressure",
        "node-allocation-cpu",
        "node-allocation-cpu-bar",
        "node-allocation-memory",
        "node-allocation-pods",
        "node-capacity",
        "node-taint-0",
        "node-system",
        "node-pods",
        "node-pod-0",
        "node-pod-1",
        "node-images",
        "node-image-0",
    ] {
        assert!(d.shown(selector), "{selector} is drawn");
    }
    assert!(!d.shown("node-panel-cordoned"), "the node is schedulable");

    let text = |d: &mut crate::detail::tests::fixture::Detail, kind: &AllocationKind| {
        d.f.vcx.update(|_, cx| {
            let panel = panel.read(cx);
            let row = panel.allocation_view()?.row(kind)?.clone();
            Some((row.requests_text(), row.limits_text()))
        })
    };
    // 250m + 750m of 4 CPU; limits 500m + 1; memory 256Mi + 1Gi of 16Gi, limits 512Mi + 2Gi.
    let (requests, limits) = text(&mut d, &AllocationKind::Cpu).expect("the pods are known");
    assert_eq!(requests, "1000m of 4000m (25%)");
    assert_eq!(limits.as_deref(), Some("1500m of 4000m (37%)"));
    let (requests, limits) = text(&mut d, &AllocationKind::Memory).unwrap();
    assert_eq!(requests, "1.3 GiB of 16 GiB (7%)");
    assert_eq!(limits.as_deref(), Some("2.5 GiB of 16 GiB (15%)"));
    let (pods, _) = text(&mut d, &AllocationKind::Pods).unwrap();
    assert_eq!(
        pods, "2 of 110 (1%)",
        "the pod of another node and the finished one are out"
    );
    let names = d.f.vcx.update(|_, cx| panel.read(cx).pod_names());
    assert_eq!(names, ["x/p1", "x/p2"]);
    assert_eq!(
        d.f.vcx.update(|_, cx| panel.read(cx).pods_are_live()),
        Some(true),
        "no list was needed: the Pod feed answered"
    );

    // The bar's fill is a quarter of its track.
    let bar = d.f.vcx.debug_bounds("node-allocation-cpu-bar").unwrap();
    let fill = d.f.vcx.debug_bounds("node-allocation-cpu-fill").unwrap();
    let share = f32::from(fill.size.width) / f32::from(bar.size.width);
    assert!((share - 0.25).abs() < 0.02, "the CPU bar is {share} full");
}

#[gpui::test]
fn the_pods_list_follows_the_cluster(cx: &mut TestAppContext) {
    let p1 = pod_on("p1", "worker-1", ("100m", "64Mi"), ("100m", "64Mi"));
    let p2 = pod_on("p2", "worker-1", ("200m", "64Mi"), ("200m", "64Mi"));
    let mut d = node_detail(cx, [worker()]);
    let resources = d.f.ports().resources;
    // The Node feed first, then the cluster-wide Pod feed the panel leases: p1 listed, p2 arrives
    // a second later, p1 goes two seconds later.
    resources.script().watch.push_ok(
        Timeline::immediate([DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
            worker(),
        ])])])
        .keep_open(),
    );
    resources.script().watch.push_ok(
        Timeline::immediate([DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
            p1.clone(),
        ])])])
        .ok_at(
            Duration::from_secs(1),
            DeltaBatch::from_deltas(vec![Delta::Applied(p2)]),
        )
        .ok_at(
            Duration::from_secs(2),
            DeltaBatch::from_deltas(vec![Delta::Deleted(p1)]),
        )
        .keep_open(),
    );
    let view = d.open(&node_ref("worker-1"));
    let panel = panel_of(&mut d, &view);
    tick(&mut d);
    let names = |d: &mut crate::detail::tests::fixture::Detail| {
        d.f.vcx.update(|_, cx| panel.read(cx).pod_names())
    };
    assert_eq!(names(&mut d), ["x/p1"]);

    resources.clock().advance(Duration::from_secs(1));
    d.settle();
    d.f.vcx
        .executor()
        .advance_clock(Duration::from_millis(1100));
    d.settle();
    assert_eq!(names(&mut d), ["x/p1", "x/p2"]);
    assert_eq!(cpu_requests(&mut d, &panel), "300m of 4000m (7%)");

    resources.clock().advance(Duration::from_secs(1));
    d.settle();
    d.f.vcx
        .executor()
        .advance_clock(Duration::from_millis(1100));
    d.settle();
    assert_eq!(names(&mut d), ["x/p2"]);
}

#[gpui::test]
fn a_status_change_of_a_pod_already_listed_reaches_the_list(cx: &mut TestAppContext) {
    let versioned = |builder: oxikube_testkit::PodBuilder, rv: &str| {
        let mut p = builder
            .namespace("x")
            .name("p1")
            .node("worker-1")
            .requests("100m", "64Mi")
            .limits("100m", "64Mi")
            .build();
        p.meta.resource_version = Some(rv.into());
        p
    };
    let mut d = node_detail(cx, [worker()]);
    let resources = d.f.ports().resources;
    resources.script().watch.push_ok(
        Timeline::immediate([DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
            worker(),
        ])])])
        .keep_open(),
    );
    // Same pod, same requests: only its status moves, twice, a second apart.
    resources.script().watch.push_ok(
        Timeline::immediate([DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
            versioned(pod().container_creating(), "1"),
        ])])])
        .ok_at(
            Duration::from_secs(1),
            DeltaBatch::from_deltas(vec![Delta::Applied(versioned(pod().running(), "2"))]),
        )
        .ok_at(
            Duration::from_secs(2),
            DeltaBatch::from_deltas(vec![Delta::Applied(versioned(pod().crash_loop(), "3"))]),
        )
        .keep_open(),
    );
    let view = d.open(&node_ref("worker-1"));
    let panel = panel_of(&mut d, &view);
    tick(&mut d);
    let statuses = |d: &mut crate::detail::tests::fixture::Detail| {
        d.f.vcx.update(|_, cx| panel.read(cx).pod_statuses())
    };
    assert_eq!(statuses(&mut d), ["x/p1 ContainerCreating"]);

    for expected in ["x/p1 Running", "x/p1 CrashLoopBackOff"] {
        resources.clock().advance(Duration::from_secs(1));
        d.settle();
        d.f.vcx
            .executor()
            .advance_clock(Duration::from_millis(1100));
        d.settle();
        assert_eq!(statuses(&mut d), [expected]);
    }
}

#[gpui::test]
fn without_the_pod_feed_the_panel_lists_the_nodes_pods_once(cx: &mut TestAppContext) {
    let mut d = node_detail(cx, [worker()]);
    let resources = d.f.ports().resources;
    resources.script().watch.push_ok(
        Timeline::immediate([DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
            worker(),
        ])])])
        .keep_open(),
    );
    // The Pod feed never lists; the field-selector list answers.
    resources
        .script()
        .watch
        .push_ok(Timeline::new().keep_open());
    resources.script().list.push_ok(ListPage::complete(vec![
        pod_on("p1", "worker-1", ("500m", "1Gi"), ("500m", "1Gi")),
        pod_on("p2", "worker-1", ("1", "1Gi"), ("1", "1Gi")),
    ]));
    let view = d.open(&node_ref("worker-1"));
    let panel = panel_of(&mut d, &view);
    assert_eq!(d.f.vcx.update(|_, cx| panel.read(cx).pods_are_live()), None);
    assert!(
        d.shown("node-allocation-pending"),
        "the bars wait for the pods"
    );

    for _ in 0..8 {
        tick(&mut d);
    }
    assert_eq!(
        d.f.vcx.update(|_, cx| panel.read(cx).pods_are_live()),
        Some(false),
        "the list answered"
    );
    assert_eq!(cpu_requests(&mut d, &panel), "1500m of 4000m (37%)");
    assert!(d.shown("node-allocation-cpu-fill"));

    let lists: Vec<_> = resources
        .recorded_calls()
        .into_iter()
        .filter_map(|call| match call {
            ResourceCall::List {
                kind,
                namespace,
                options,
            } if kind.kind.as_ref() == "Pod" => Some((namespace, options.field_selector)),
            _ => None,
        })
        .collect();
    assert_eq!(
        lists,
        [(
            None,
            Some("spec.nodeName=worker-1,status.phase!=Succeeded,status.phase!=Failed".to_owned())
        )],
        "one list, for this node's pods only"
    );
}

#[gpui::test]
fn a_forbidden_list_says_why_the_pods_are_missing(cx: &mut TestAppContext) {
    let mut d = node_detail(cx, [worker()]);
    let resources = d.f.ports().resources;
    resources.script().watch.push_ok(
        Timeline::immediate([DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
            worker(),
        ])])])
        .keep_open(),
    );
    resources
        .script()
        .watch
        .push_ok(Timeline::new().keep_open());
    resources
        .script()
        .list
        .push_err(OxiError::forbidden("pods is forbidden"));
    let view = d.open(&node_ref("worker-1"));
    let panel = panel_of(&mut d, &view);
    for _ in 0..8 {
        tick(&mut d);
    }
    assert_eq!(
        d.f.vcx
            .update(|_, cx| panel.read(cx).fetch_error().map(str::to_owned)),
        Some("pods is forbidden".to_owned())
    );
    assert!(d.shown("node-panel"), "the rest of the panel still draws");
    assert!(d.shown("node-allocation-pending"));
}

#[gpui::test]
fn taints_conditions_and_the_cordon_badge_follow_the_node(cx: &mut TestAppContext) {
    let cordoned = node()
        .name("worker-1")
        .cordoned()
        .condition("MemoryPressure", true)
        .build();
    let mut d = node_detail(cx, [cordoned]);
    let view = d.open(&node_ref("worker-1"));
    let _panel = panel_of(&mut d, &view);
    assert!(d.shown("node-panel-cordoned"), "the badge");
    assert!(d.shown("node-taint-0"), "the unschedulable taint");
    assert!(
        d.shown("node-panel-uncordon") && !d.shown("node-panel-cordon"),
        "a cordoned node offers uncordon"
    );
    assert!(d.shown("node-condition-MemoryPressure"));
}

#[gpui::test]
fn the_button_sends_cordon_or_uncordon_through_the_dispatcher(cx: &mut TestAppContext) {
    let mut d = node_detail(cx, [worker(), node().name("worker-2").cordoned().build()]);
    let view = d.open(&node_ref("worker-1"));
    let _panel = panel_of(&mut d, &view);
    d.f.dispatcher.clear();
    d.click("node-panel-cordon");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::NodeCordon {
            target: node_ref("worker-1")
        }]
    );

    let other = d.open(&node_ref("worker-2"));
    let _panel = panel_of(&mut d, &other);
    d.f.dispatcher.clear();
    d.click("node-panel-uncordon");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::NodeUncordon {
            target: node_ref("worker-2")
        }]
    );
}

#[gpui::test]
fn a_read_only_cluster_disables_the_button(cx: &mut TestAppContext) {
    let mut d = node_detail(cx, [worker()]);
    d.f.sessions
        .set_read_only(&crate::table::tests::fixture::cluster(), true)
        .expect("open session");
    let view = d.open(&node_ref("worker-1"));
    let panel = panel_of(&mut d, &view);
    tick(&mut d);
    let reason =
        d.f.vcx
            .update(|_, cx| panel.read(cx).blocked().map(str::to_owned))
            .expect("blocked");
    assert!(reason.contains("read-only"), "{reason}");
    d.f.dispatcher.clear();
    d.draw();
    if let Some(bounds) = d.f.vcx.debug_bounds("node-panel-cordon") {
        d.f.vcx.simulate_click(bounds.center(), Default::default());
        d.settle();
    }
    assert!(
        d.f.dispatcher.sent().is_empty(),
        "a disabled button sends nothing"
    );
}

#[gpui::test]
fn a_pod_row_opens_the_pod(cx: &mut TestAppContext) {
    let mut d = node_detail(
        cx,
        [
            worker(),
            pod_on("p1", "worker-1", ("100m", "64Mi"), ("100m", "64Mi")),
        ],
    );
    let view = d.open(&node_ref("worker-1"));
    let _panel = panel_of(&mut d, &view);
    tick(&mut d);
    d.f.dispatcher.clear();
    // The kind's panel scrolls inside its own box, and the pods are below the fold.
    d.draw();
    let panel_box =
        d.f.vcx
            .debug_bounds("detail-kind-panel")
            .expect("the panel is drawn");
    d.f.vcx.simulate_event(gpui::ScrollWheelEvent {
        position: panel_box.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-260.))),
        modifiers: gpui::Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    d.settle();
    d.draw();
    d.click("node-pod-0");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::ResourceOpen {
            target: oxikube_domain::ids::ResourceRef::namespaced(
                crate::table::tests::fixture::cluster(),
                oxikube_domain::ids::Gvk::new("", "v1", "Pod"),
                "x",
                "p1",
            )
        }]
    );
}

#[gpui::test]
fn another_kind_has_no_node_panel(cx: &mut TestAppContext) {
    let mut d = node_detail(cx, [crate::detail::tests::fixture::web_pod()]);
    let _ = d.open(&crate::detail::tests::fixture::pod_ref("web-0"));
    d.draw();
    assert!(!d.shown("node-panel"));
}
