//! The workload panel in the detail drawer: replicas, strategy, selector, pod template, the
//! ReplicaSets and Pods the workload owns (found by `ownerReferences`), and the Scale button.

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;

use super::{
    daemonsets, pods, replicasets, statefulsets, web_deployment, web_pod, web_ref, web_replicaset,
};
use crate::detail::tests::fixture::Detail;
use crate::kinds::{ScaleDialog, WorkloadPanel};
use crate::table::tests::fixture::cluster;

fn objects() -> Vec<oxikube_domain::Resource> {
    vec![
        web_deployment(),
        // The current revision, the old one scaled down, and a ReplicaSet of another Deployment.
        web_replicaset("web-5d", "rs-new", 3, 2, "dep-uid"),
        web_replicaset("web-4c", "rs-old", 0, 0, "dep-uid"),
        web_replicaset("api-7f", "rs-api", 2, 2, "api-uid"),
        web_pod("web-5d-aaaaa", "rs-new", "web"),
        web_pod("web-5d-bbbbb", "rs-new", "web"),
        // Matches the selector, but belongs to another ReplicaSet: not this Deployment's.
        web_pod("stray-xxxxx", "rs-other", "web"),
    ]
}

fn open_panel(
    d: &mut Detail,
    target: &ResourceRef,
) -> (
    gpui::Entity<crate::detail::DetailView>,
    gpui::Entity<WorkloadPanel>,
) {
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    let view = d.open(target);
    d.draw();
    let panel = d
        .read(&view, |v| v.kind_panel().cloned())
        .expect("the kind panel")
        .downcast::<WorkloadPanel>()
        .expect("a workload panel");
    d.settle();
    (view, panel)
}

fn window(cx: &mut TestAppContext) -> Detail {
    Detail::with_workloads(
        cx,
        [statefulsets(), replicasets(), daemonsets(), pods()],
        objects(),
    )
}

#[gpui::test]
fn the_panel_shows_replicas_strategy_selector_and_the_pod_template(cx: &mut TestAppContext) {
    let mut d = window(cx);
    let (_, panel) = open_panel(&mut d, &web_ref());
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
            ("Ready", "2/3"),
            ("Up-to-date", "3"),
            ("Available", "2"),
            ("Replicas", "3")
        ]
    );
    assert_eq!(
        model.strategy.as_deref(),
        Some("RollingUpdate · Max surge 25% · Max unavailable 1")
    );
    assert_eq!(model.selector, "app=web");
    assert_eq!(model.containers.len(), 1);
    assert_eq!(&*model.containers[0].image, "shop/web:2.1");
    assert_eq!(model.containers[0].ports, "http:8080/TCP");
    assert_eq!(
        model.containers[0].resources,
        "requests cpu 100m · limits memory 256Mi"
    );
    for part in [
        "workload-panel",
        "workload-stats",
        "workload-strategy",
        "workload-selector",
        "workload-template",
        "workload-container-0",
        "workload-scale",
    ] {
        assert!(d.shown(part), "{part} is drawn");
    }
    assert!(
        d.shown("detail-overview"),
        "the generic rows (conditions) stay below"
    );
}

#[gpui::test]
fn the_panel_lists_the_owned_replicasets_current_first_and_the_owned_pods_only(
    cx: &mut TestAppContext,
) {
    let mut d = window(cx);
    let (_, panel) = open_panel(&mut d, &web_ref());
    d.settle();
    let (sets, pods) = d.f.vcx.update(|_, cx| {
        let p = panel.read(cx);
        (p.replicaset_rows().clone(), p.pod_rows().clone())
    });
    let names = |rows: &crate::kinds::workloads::panel::Owned| -> Vec<String> {
        rows.rows
            .iter()
            .map(|r| r.target.name.to_string())
            .collect()
    };
    assert_eq!(
        names(&sets),
        ["web-5d", "web-4c"],
        "api-7f belongs to another Deployment"
    );
    assert_eq!(sets.rows[0].status, "2/3");
    assert_eq!(sets.rows[1].info, "scaled down");
    assert_eq!(
        names(&pods),
        ["web-5d-aaaaa", "web-5d-bbbbb"],
        "the stray pod matches the selector but is owned by another ReplicaSet"
    );
    assert_eq!(pods.rows[0].status, "Running");
    assert!(d.shown("workload-replicasets"));
    assert!(d.shown("workload-pods"));
    assert!(d.shown("workload-replicasets-row-0"));
    assert!(d.shown("workload-pods-row-1"));
}

#[gpui::test]
fn an_owned_object_is_a_link_to_its_detail(cx: &mut TestAppContext) {
    let mut d = window(cx);
    let (_, _panel) = open_panel(&mut d, &web_ref());
    d.f.dispatcher.clear();
    d.click("workload-replicasets-row-0");
    let sent = d.f.dispatcher.sent();
    assert!(
        matches!(sent.as_slice(), [Command::ResourceOpen { target }]
            if &*target.name == "web-5d" && &*target.gvk.kind == "ReplicaSet"
                && target.namespace() == Some("shop")),
        "{sent:?}"
    );
    d.f.dispatcher.clear();
    d.click("workload-pods-row-0");
    let sent = d.f.dispatcher.sent();
    assert!(
        matches!(sent.as_slice(), [Command::ResourceOpen { target }]
            if &*target.name == "web-5d-aaaaa" && target.gvk.is_pod()),
        "{sent:?}"
    );
}

#[gpui::test]
fn a_replicaset_lists_its_pods_and_has_no_strategy_line(cx: &mut TestAppContext) {
    let mut d = window(cx);
    let target = ResourceRef::namespaced(
        cluster(),
        oxikube_domain::ids::Gvk::new("apps", "v1", "ReplicaSet"),
        "shop",
        "web-5d",
    );
    let (_, panel) = open_panel(&mut d, &target);
    d.settle();
    let (model, sets, pods) = d.f.vcx.update(|_, cx| {
        let p = panel.read(cx);
        (
            p.model().cloned().unwrap(),
            p.replicaset_rows().total,
            p.pod_rows().total,
        )
    });
    assert_eq!(model.strategy, None);
    assert_eq!(model.stats[0].label, "Ready");
    assert_eq!(sets, 0, "a ReplicaSet owns no ReplicaSets");
    assert_eq!(pods, 2);
    assert!(!d.shown("workload-strategy"));
    assert!(!d.shown("workload-replicasets"));
}

#[gpui::test]
fn the_scale_button_opens_the_dialog_and_read_only_disables_it(cx: &mut TestAppContext) {
    let mut d = window(cx);
    let (_, panel) = open_panel(&mut d, &web_ref());
    d.f.ports()
        .resources
        .script()
        .get_scale
        .push_ok(oxikube_ports::Scale {
            replicas: 3,
            current_replicas: 2,
            selector: None,
            resource_version: None,
        });
    d.click("workload-scale");
    let tabs = d.f.tabs.clone();
    let open = d.f.vcx.update(|_, cx| {
        let tab = tabs.read(cx).tab(&cluster()).cloned().unwrap();
        let layer = tab.read(cx).workspace().read(cx).modal_layer().clone();
        layer.read(cx).active_modal::<ScaleDialog>().is_some()
    });
    assert!(open, "the dialog is open");

    // Read-only: the button is disabled with the reason, and clicking does nothing.
    d.f.vcx.update(|window, cx| {
        let tab = tabs.read(cx).tab(&cluster()).cloned().unwrap();
        let ws = tab.read(cx).workspace().clone();
        ws.update(cx, |ws, cx| ws.hide_modal(window, cx));
    });
    d.f.sessions.set_read_only(&cluster(), true).unwrap();
    d.settle();
    let state =
        d.f.vcx
            .update(|_, cx| panel.read(cx).scale_state_for_test(cx));
    assert!(
        state
            .expect("offered")
            .expect("blocked")
            .contains("read-only")
    );
}

#[gpui::test]
fn the_palette_runs_scale_through_the_focused_detail(cx: &mut TestAppContext) {
    use oxikube_workspace::command_surface::CommandSurface as _;
    let mut d = window(cx);
    let (view, _panel) = open_panel(&mut d, &web_ref());
    let own = d.f.vcx.update(|_, cx| view.read(cx).own_commands(cx));
    assert_eq!(own, [oxikube_domain::command::CommandId::WORKLOAD_SCALE]);
    d.f.ports()
        .resources
        .script()
        .get_scale
        .push_ok(oxikube_ports::Scale {
            replicas: 3,
            current_replicas: 3,
            selector: None,
            resource_version: None,
        });
    let ran = d.f.vcx.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.run_command(
                oxikube_domain::command::CommandId::WORKLOAD_SCALE,
                vec![web_ref()],
                window,
                cx,
            )
        })
    });
    assert!(ran);
    d.settle();
    let tabs = d.f.tabs.clone();
    let open = d.f.vcx.update(|_, cx| {
        let tab = tabs.read(cx).tab(&cluster()).cloned().unwrap();
        let layer = tab.read(cx).workspace().read(cx).modal_layer().clone();
        layer.read(cx).active_modal::<ScaleDialog>().is_some()
    });
    assert!(open);
    // Read-only: not offered at all.
    d.f.sessions.set_read_only(&cluster(), true).unwrap();
    d.settle();
    let own = d.f.vcx.update(|_, cx| view.read(cx).own_commands(cx));
    assert!(own.is_empty());
}
