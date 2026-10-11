//! The Service's detail (E12-S13), `#[gpui::test]`s over the testkit fakes: the panel lists the
//! ports, the EndpointSlices and the backing pods, queries them by label on the server, explains a
//! Service that selects nothing, hides "Open in browser" without a hook, and navigates to the pods.

use gpui::{Entity, TestAppContext};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::view::NoSelector;
use oxikube_ports::WatchOptions;
use oxikube_testkit::{ResourceCall, resource};
use serde_json::{Value, json};

use super::model::{Pods, Slices};
use super::{ServiceBrowser, ServicePanel};
use crate::detail::DetailView;
use crate::detail::tests::fixture::Detail;
use crate::table::tests::fixture::cluster;

mod verbs;

fn service_ref(name: &str) -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Service"), "shop", name)
}

fn service(name: &str, spec: Value) -> Resource {
    let mut r = resource("v1", "Service")
        .name(name)
        .namespace("shop")
        .field("spec", spec)
        .field("status", json!({"loadBalancer": {}}))
        .created("2026-01-01T00:00:00Z")
        .build();
    r.meta.resource_version = Some("5".into());
    r
}

fn web_service() -> Resource {
    service(
        "web",
        json!({
            "type": "ClusterIP", "clusterIP": "10.96.0.10",
            "selector": {"app": "web"},
            "ports": [
                {"name": "http", "port": 80, "targetPort": "web"},
                {"name": "dns", "port": 53, "protocol": "UDP", "targetPort": 53},
            ],
        }),
    )
}

fn slice(name: &str, service: &str, endpoints: Value) -> Resource {
    let mut r = resource("discovery.k8s.io/v1", "EndpointSlice")
        .name(name)
        .namespace("shop")
        .label("kubernetes.io/service-name", service)
        .field("addressType", json!("IPv4"))
        .field(
            "ports",
            json!([{"name": "http", "port": 8080, "protocol": "TCP"}]),
        )
        .field("endpoints", endpoints)
        .build();
    r.meta.resource_version = Some("1".into());
    r
}

fn backing_pod(name: &str, app: &str) -> Resource {
    let mut r = oxikube_testkit::pod()
        .namespace("shop")
        .name(name)
        .label("app", app)
        .running()
        .ip("10.1.0.5")
        .node("worker-1")
        .build();
    let mut json = r.to_value();
    json["spec"]["containers"] = json!([{
        "name": "app", "image": "app:1", "ports": [{"name": "web", "containerPort": 8080}]
    }]);
    r = Resource::from_json(json).expect("a pod");
    r.meta.resource_version = Some("1".into());
    r
}

/// The cluster of the shop: the `web` Service, two of its three slices' worth of endpoints, and
/// pods of `web` and of `db`.
fn shop() -> Vec<Resource> {
    vec![
        web_service(),
        slice(
            "web-abc",
            "web",
            json!([
                {"addresses": ["10.1.0.5"], "conditions": {"ready": true}},
                {"addresses": ["10.1.0.6"], "conditions": {"ready": false}},
            ]),
        ),
        slice("web-def", "web", json!([{"addresses": ["10.1.0.7"]}])),
        slice("api-abc", "api", json!([{"addresses": ["10.9.0.1"]}])),
        backing_pod("web-0", "web"),
        backing_pod("web-1", "web"),
        backing_pod("db-0", "db"),
    ]
}

fn detail(cx: &mut TestAppContext, objects: Vec<Resource>) -> Detail {
    let mut d = Detail::new(cx, objects);
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    d
}

/// The Service panel of `view`.
fn service_panel(d: &mut Detail, view: &Entity<DetailView>) -> Entity<ServicePanel> {
    d.draw();
    let any = d
        .read(view, |v| v.kind_panel().cloned())
        .expect("the Service has its own panel");
    any.downcast::<ServicePanel>().expect("a Service panel")
}

fn watches(d: &mut Detail) -> Vec<(String, Option<String>, Option<String>)> {
    d.f.ports()
        .resources
        .recorded_calls()
        .into_iter()
        .filter_map(|call| match call {
            ResourceCall::Watch {
                kind,
                namespace,
                options: WatchOptions { label_selector, .. },
            } => Some((kind.kind.to_string(), namespace, label_selector)),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn the_detail_lists_ports_slices_and_backing_pods(cx: &mut TestAppContext) {
    let mut d = detail(cx, shop());
    let view = d.open(&service_ref("web"));
    d.draw();
    for selector in [
        "detail-kind-panel",
        "service-panel",
        "service-port-0",
        "service-port-1",
        "service-slice-0",
        "service-slice-1",
        "service-pod-0",
        "service-pod-1",
        "service-show-pods",
        "service-health",
    ] {
        assert!(d.shown(selector), "{selector} is on screen");
    }
    assert!(
        !d.shown("service-slice-2"),
        "the other service's slice is not listed"
    );
    assert!(!d.shown("service-pod-2"), "db-0 is not selected");
    assert!(
        d.shown("detail-overview"),
        "the generic Overview stays below the panel"
    );

    let panel = service_panel(&mut d, &view);
    let model =
        d.f.vcx
            .update(|_, cx| panel.read(cx).view().cloned())
            .expect("built");
    assert_eq!(model.ports[0].text, "80/TCP");
    assert_eq!(
        model.ports[0].target, "web → 8080",
        "the named target port is resolved against a backing pod"
    );
    assert!(model.ports[0].browsable);
    assert!(
        !model.ports[1].browsable,
        "UDP cannot be opened in a browser"
    );
    match &model.slices {
        Slices::Listed { rows, counts, .. } => {
            assert_eq!(rows.len(), 2);
            assert_eq!((counts.ready, counts.not_ready), (2, 1));
        }
        other => panic!("slices: {other:?}"),
    }
    match &model.pods {
        Pods::Listed { rows, total, .. } => {
            assert_eq!(*total, 2);
            assert_eq!(rows[0].name.as_ref(), "web-0");
            assert_eq!(rows[0].status, "Running");
            assert_eq!(rows[0].ip, "10.1.0.5");
        }
        other => panic!("pods: {other:?}"),
    }
    assert_eq!(
        model.health.as_ref().map(|(text, _)| text.as_str()),
        Some("Endpoints: 2 ready, 1 not ready")
    );
}

#[gpui::test]
fn the_detail_queries_by_label_on_the_server_and_starts_no_other_feed(cx: &mut TestAppContext) {
    let mut d = detail(cx, shop());
    d.open(&service_ref("web"));
    let calls = watches(&mut d);
    let namespaced = |kind: &str, selector: Option<&str>| {
        (
            kind.to_owned(),
            Some("shop".to_owned()),
            selector.map(str::to_owned),
        )
    };
    assert!(
        calls.contains(&namespaced(
            "EndpointSlice",
            Some("kubernetes.io/service-name=web")
        )),
        "one slice query by service name: {calls:?}"
    );
    assert!(
        calls.contains(&namespaced("Pod", Some("app=web"))),
        "one pod query by the selector: {calls:?}"
    );
    assert_eq!(
        calls.iter().filter(|(kind, ..)| kind == "Pod").count(),
        1,
        "no unfiltered pod feed"
    );
    assert_eq!(
        calls
            .iter()
            .filter(|(kind, ..)| kind == "EndpointSlice")
            .count(),
        1
    );
}

#[gpui::test]
fn closing_the_detail_stops_all_three_feeds(cx: &mut TestAppContext) {
    let mut d = detail(cx, shop());
    let view = d.open(&service_ref("web"));
    d.draw();
    assert_eq!(
        d.f.ports().resources.live_watches(),
        3,
        "the Service itself, its slices and its pods"
    );
    d.click("detail-close");
    d.f.clock.advance(std::time::Duration::from_secs(60));
    d.settle();
    assert_eq!(
        d.f.ports().resources.live_watches(),
        0,
        "the panel's feeds go with the drawer's, though the test still holds the view"
    );
    drop(view);
}

#[gpui::test]
fn a_service_without_a_selector_says_why_and_starts_no_pod_feed(cx: &mut TestAppContext) {
    let manual = service("manual", json!({"ports": [{"port": 80}]}));
    let mut d = detail(cx, vec![manual, slice("manual-1", "manual", json!([]))]);
    let view = d.open(&service_ref("manual"));
    d.draw();
    assert!(d.shown("service-no-pods"));
    assert!(!d.shown("service-show-pods"), "nothing to show");
    let panel = service_panel(&mut d, &view);
    let model =
        d.f.vcx
            .update(|_, cx| panel.read(cx).view().cloned())
            .expect("built");
    assert_eq!(model.pods, Pods::None(NoSelector::Manual));
    assert!(
        watches(&mut d).iter().all(|(kind, ..)| kind != "Pod"),
        "no pod feed for a selectorless service"
    );

    let external = service(
        "db",
        json!({"type": "ExternalName", "externalName": "db.example.com"}),
    );
    let mut d = detail(cx, vec![external]);
    let view = d.open(&service_ref("db"));
    let panel = service_panel(&mut d, &view);
    let model =
        d.f.vcx
            .update(|_, cx| panel.read(cx).view().cloned())
            .expect("built");
    assert_eq!(model.pods, Pods::None(NoSelector::ExternalName));
    assert!(d.shown("service-no-pods"));
}

#[gpui::test]
fn open_in_browser_is_hidden_without_a_hook_and_offered_for_tcp_ports_with_one(
    cx: &mut TestAppContext,
) {
    let mut d = detail(cx, shop());
    d.open(&service_ref("web"));
    assert!(
        !d.shown("service-open-80"),
        "no port-forward manager: no button"
    );
    assert!(d.shown("service-port-0"), "the ports themselves are listed");

    d.f.vcx.update(|_, cx| ServiceBrowser::enable(cx));
    d.f.vcx.update(|window, _| window.refresh());
    assert!(d.shown("service-open-80"), "TCP port 80 can be opened");
    assert!(!d.shown("service-open-53"), "UDP port 53 cannot");

    d.f.dispatcher.clear();
    d.click("service-open-80");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::ServiceOpenInBrowser {
            target: service_ref("web"),
            port: Some(80),
        }],
        "the button is the command, with the port"
    );

    d.f.vcx.update(|_, cx| ServiceBrowser::disable(cx));
    d.f.vcx.update(|window, _| window.refresh());
    assert!(!d.shown("service-open-80"));
}

#[gpui::test]
fn show_pods_opens_the_pod_table_filtered_by_the_selector(cx: &mut TestAppContext) {
    let mut d = detail(cx, shop());
    d.open(&service_ref("web"));
    d.f.dispatcher.clear();
    d.click("service-show-pods");
    d.settle();

    let sent = d.f.dispatcher.sent();
    assert_eq!(
        sent[0],
        Command::ServiceShowPods {
            target: service_ref("web")
        }
    );
    assert!(
        sent.contains(&Command::ResourceOpenList {
            cluster: cluster(),
            gvk: Gvk::new("", "v1", "Pod"),
        }),
        "{sent:?}"
    );
    assert!(
        sent.contains(&Command::TableSetFilter {
            cluster: cluster(),
            gvk: Gvk::new("", "v1", "Pod"),
            text: "-l app=web".to_owned(),
        }),
        "{sent:?}"
    );

    // The table opened in the cluster's tab, filtered on the server by the selector.
    let workspace = d.workspace();
    let tables = d.f.vcx.update(|_, cx| {
        workspace
            .read(cx)
            .items_of_type::<crate::table::ResourceTable>()
    });
    let pods: Vec<_> = tables
        .into_iter()
        .filter(|t| d.f.vcx.update(|_, cx| t.read(cx).gvk().is_pod()))
        .collect();
    assert_eq!(pods.len(), 1, "the Pods table is open");
    let selector = d.f.vcx.update(|_, cx| {
        pods[0]
            .read(cx)
            .filter_parts()
            .selector
            .as_ref()
            .map(ToString::to_string)
    });
    assert_eq!(selector.as_deref(), Some("app=web"));
    assert_eq!(d.f.names(&pods[0]), ["web-0", "web-1"]);
}

#[gpui::test]
fn a_pod_row_and_a_slice_row_open_their_details(cx: &mut TestAppContext) {
    let mut d = detail(cx, shop());
    d.open(&service_ref("web"));
    d.f.dispatcher.clear();
    d.click("service-pod-1");
    let pod = ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "shop", "web-1");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::ResourceOpen {
            target: pod.clone()
        }]
    );
    assert_eq!(
        d.drawer_view().map(|v| d.read(&v, |v| v.target().clone())),
        Some(pod),
        "the drawer now shows the pod"
    );

    d.open(&service_ref("web"));
    d.f.dispatcher.clear();
    d.click("service-slice-0");
    let slice = ResourceRef::namespaced(
        cluster(),
        Gvk::new("discovery.k8s.io", "v1", "EndpointSlice"),
        "shop",
        "web-abc",
    );
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::ResourceOpen { target: slice }]
    );
}

#[gpui::test]
fn drawing_frames_builds_nothing(cx: &mut TestAppContext) {
    let mut d = detail(cx, shop());
    let view = d.open(&service_ref("web"));
    let panel = service_panel(&mut d, &view);
    let before = d.f.vcx.update(|_, cx| panel.read(cx).rebuilds);
    for _ in 0..20 {
        d.draw();
    }
    let after = d.f.vcx.update(|_, cx| panel.read(cx).rebuilds);
    assert_eq!(
        before, after,
        "the model is built per delta, never per frame"
    );
}

#[gpui::test]
fn a_new_backing_pod_and_a_new_slice_reach_the_open_panel(cx: &mut TestAppContext) {
    use std::time::Duration;

    use oxikube_ports::{Delta, DeltaBatch};
    use oxikube_testkit::Timeline;

    let mut d = detail(cx, vec![]);
    // Watches start in this order: the Service, then the panel's slices, then its pods.
    let batch = |deltas| DeltaBatch::from_deltas(deltas);
    let at = |secs, deltas| (Duration::from_secs(secs), batch(deltas));
    let feeds = [
        vec![at(0, vec![Delta::Restarted(vec![web_service()])])],
        vec![
            at(
                0,
                vec![Delta::Restarted(vec![slice(
                    "web-abc",
                    "web",
                    json!([{"addresses": ["10.1.0.5"]}]),
                )])],
            ),
            at(
                5,
                vec![Delta::Applied(slice(
                    "web-def",
                    "web",
                    json!([{"addresses": ["10.1.0.7"]}]),
                ))],
            ),
        ],
        vec![
            at(0, vec![Delta::Restarted(vec![backing_pod("web-0", "web")])]),
            at(5, vec![Delta::Applied(backing_pod("web-1", "web"))]),
        ],
    ];
    for events in feeds {
        let timeline = events
            .into_iter()
            .fold(Timeline::new(), |t, (when, batch)| t.ok_at(when, batch))
            .keep_open();
        d.f.ports().resources.script().watch.push_ok(timeline);
    }
    let view = d.open(&service_ref("web"));
    let panel = service_panel(&mut d, &view);
    let counts = |d: &mut Detail, panel: &Entity<ServicePanel>| {
        d.f.vcx.update(|_, cx| {
            let model = panel.read(cx).view().cloned().expect("built");
            let slices = match model.slices {
                Slices::Listed { rows, .. } => rows.len(),
                _ => 0,
            };
            let pods = match model.pods {
                Pods::Listed { total, .. } => total,
                _ => 0,
            };
            (slices, pods)
        })
    };
    assert_eq!(counts(&mut d, &panel), (1, 1));

    d.f.ports()
        .resources
        .clock()
        .advance(Duration::from_secs(5));
    d.settle();
    assert_eq!(
        counts(&mut d, &panel),
        (2, 2),
        "both lists followed their watches"
    );
}

#[test]
fn long_lists_are_capped_and_the_counts_cover_everything() {
    use std::sync::Arc;

    use oxikube_app::CoreColumns;
    use oxikube_app::store::StoreObject;
    use oxikube_domain::view::ServiceSummary;

    use super::model::{FeedStatus, Inputs, MAX_PODS, MAX_SLICES, ServiceView};

    let summary = ServiceSummary::from_resource(&web_service()).unwrap();
    let pods: Vec<Arc<StoreObject>> = (0..120)
        .map(|i| {
            let name = format!("web-{i:03}");
            Arc::new(StoreObject::Resource(backing_pod(&name, "web")))
        })
        .collect();
    let slices: Vec<Arc<StoreObject>> = (0..30)
        .map(|i| {
            let name = format!("web-{i:02}");
            let endpoints = json!([
                {"addresses": ["10.0.0.1"], "conditions": {"ready": true}},
                {"addresses": ["10.0.0.2"], "conditions": {"ready": false}},
            ]);
            Arc::new(StoreObject::Resource(slice(&name, "web", endpoints)))
        })
        .collect();
    let target = service_ref("web");
    let columns = CoreColumns::new();
    let view = ServiceView::build(&Inputs {
        service: &summary,
        target: &target,
        slices: &slices,
        slices_status: &FeedStatus::Ready,
        pods: Some(&pods),
        pods_status: &FeedStatus::Ready,
        columns: &columns,
        now: "2026-01-10T00:00:00Z".parse().unwrap(),
    });
    match view.pods {
        Pods::Listed { rows, more, total } => {
            assert_eq!(rows.len(), MAX_PODS);
            assert_eq!((more, total), (120 - MAX_PODS, 120));
        }
        other => panic!("pods: {other:?}"),
    }
    match view.slices {
        Slices::Listed { rows, more, counts } => {
            assert_eq!(rows.len(), MAX_SLICES);
            assert_eq!(more, 30 - MAX_SLICES);
            assert_eq!(
                (counts.ready, counts.not_ready),
                (30, 30),
                "every slice is counted"
            );
        }
        other => panic!("slices: {other:?}"),
    }
    assert_eq!(
        view.ports[0].target, "web → 8080",
        "the named port resolves against the pods the panel holds"
    );
}

#[test]
fn a_feed_that_cannot_be_read_says_so_instead_of_an_empty_list() {
    use oxikube_app::CoreColumns;
    use oxikube_domain::view::ServiceSummary;

    use super::model::{FeedStatus, Inputs, ServiceView};

    let summary = ServiceSummary::from_resource(&web_service()).unwrap();
    let target = service_ref("web");
    let columns = CoreColumns::new();
    let forbidden = FeedStatus::Failed("endpointslices is forbidden".into());
    let view = ServiceView::build(&Inputs {
        service: &summary,
        target: &target,
        slices: &[],
        slices_status: &forbidden,
        pods: Some(&[]),
        pods_status: &FeedStatus::Loading,
        columns: &columns,
        now: "2026-01-10T00:00:00Z".parse().unwrap(),
    });
    assert_eq!(
        view.slices,
        Slices::Failed("endpointslices is forbidden".into())
    );
    assert_eq!(view.pods, Pods::Loading);
    assert_eq!(view.health, None, "no health claim without the slices");
}

#[gpui::test]
fn services_are_registered_under_network_with_their_panel_and_actions(cx: &mut TestAppContext) {
    use oxikube_app::Section;
    use oxikube_domain::command::CommandId;

    use crate::kinds::KindSpecs;

    let mut d = detail(cx, vec![]);
    let gvk = Gvk::new("", "v1", "Service");
    d.f.vcx.update(|_, cx| {
        let meta = KindSpecs::kind_for(cx, &gvk);
        assert_eq!(
            meta.sidebar.map(|slot| slot.section),
            Some(Section::NETWORK)
        );
        assert_eq!(
            meta.actions,
            [
                CommandId::SERVICE_SHOW_PODS,
                CommandId::SERVICE_OPEN_IN_BROWSER
            ]
        );
        assert!(
            !KindSpecs::detail_for(cx, &gvk).is_generic(),
            "the panel is registered"
        );
        // Another Network kind keeps the generic detail until its own story.
        let ingress = Gvk::new("networking.k8s.io", "v1", "Ingress");
        assert!(KindSpecs::detail_for(cx, &ingress).is_generic());
    });
}

#[gpui::test]
fn a_user_reaches_the_service_from_the_network_section_of_the_sidebar(cx: &mut TestAppContext) {
    use oxikube_app::ColumnId;
    use oxikube_domain::access::AccessRules;

    use crate::kinds::tests::sidebar_panel;
    use crate::table::ResourceTable;

    let mut d = detail(cx, shop());
    d.f.ports().access.set_rules(AccessRules::all_access());
    d.settle();

    // Network -> Services opens the table with the Service columns.
    let panel = sidebar_panel(&mut d.f);
    d.f.vcx
        .update(|_, cx| panel.update(cx, |panel, cx| panel.activate("network/services", cx)));
    d.settle();
    let workspace = d.workspace();
    let tables: Vec<Entity<ResourceTable>> =
        d.f.vcx
            .update(|_, cx| workspace.read(cx).items_of_type::<ResourceTable>());
    assert_eq!(
        tables.len(),
        1,
        "the sidebar entry opened the Services table"
    );
    let columns = d.f.vcx.update(|_, cx| {
        tables[0]
            .read(cx)
            .read_rows(cx, |rows| rows.layout().visible_ids())
    });
    for id in [
        "type",
        "cluster-ip",
        "external-ip",
        "ports",
        "selector",
        "status",
    ] {
        assert!(columns.contains(&ColumnId::new(id)), "{id} in {columns:?}");
    }
    assert_eq!(d.f.names(&tables[0]), ["web"]);

    // Enter on the row opens the detail with the Service panel.
    d.f.keys(&tables[0], "j enter");
    d.settle();
    assert!(
        d.shown("service-panel"),
        "the drawer shows the Service's panel"
    );
}
