//! The detail drawer of a Service (E12-S13), in the dark and the light theme: facts and selector
//! chips, ports with the named target port resolved and "Open in browser" on the TCP ports (a
//! port-forward manager is installed), the EndpointSlices with ready counts, and the backing pods
//! (one of them crash-looping).

use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, px, size};
use jiff::Timestamp;
use oxikube_app::store::ResourceStores;
use oxikube_app::{ClusterSessionManager, CoreColumns};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName, ResourceRef};
use oxikube_ports::{ClockPort, ClusterContext, SourceId};
use oxikube_resources_ui::detail::{DetailDeps, DetailView, Mount};
use oxikube_resources_ui::kinds::service::ServiceBrowser;
use oxikube_resources_ui::table::store_runtime;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, pod, resource,
    screenshot::RgbaImage,
};
use serde_json::{Value, json};

use super::{Ignore, check, headless};

const WIDTH: f32 = 520.0;
const HEIGHT: f32 = 820.0;

fn web_service() -> Resource {
    let mut r = resource("v1", "Service")
        .name("web")
        .namespace("shop")
        .label("app", "web")
        .created("2026-01-01T00:00:00Z")
        .field(
            "spec",
            json!({
                "type": "ClusterIP", "clusterIP": "10.96.12.34", "clusterIPs": ["10.96.12.34"],
                "ipFamilies": ["IPv4"], "sessionAffinity": "None",
                "selector": {"app": "web", "tier": "frontend"},
                "ports": [
                    {"name": "http", "port": 80, "protocol": "TCP", "targetPort": "web"},
                    {"name": "https", "port": 443, "protocol": "TCP", "targetPort": 8443},
                    {"name": "dns", "port": 53, "protocol": "UDP", "targetPort": 53},
                ],
            }),
        )
        .field("status", json!({"loadBalancer": {}}))
        .build();
    r.meta.resource_version = Some("5".into());
    r
}

fn slice(name: &str, endpoints: Value) -> Resource {
    let mut r = resource("discovery.k8s.io/v1", "EndpointSlice")
        .name(name)
        .namespace("shop")
        .label("kubernetes.io/service-name", "web")
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

fn backing(name: &str, ip: &str, crashing: bool) -> Resource {
    let builder = pod()
        .namespace("shop")
        .name(name)
        .label("app", "web")
        .label("tier", "frontend")
        .node("kind-worker")
        .ip(ip);
    let builder = if crashing {
        builder.crash_loop().restarts(6)
    } else {
        builder.running()
    };
    let mut json = builder.json();
    json["metadata"]["resourceVersion"] = json!("3");
    json["spec"]["containers"] = json!([{
        "name": "app", "image": "app:1", "ports": [{"name": "web", "containerPort": 8080}]
    }]);
    Resource::from_json(json).expect("a pod")
}

fn render(light: bool) -> anyhow::Result<RgbaImage> {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let ports = connector.ports_for(&cluster);
    for object in [
        web_service(),
        slice(
            "web-abc12",
            json!([
                {"addresses": ["10.244.0.12"], "conditions": {"ready": true}},
                {"addresses": ["10.244.0.13"], "conditions": {"ready": true}},
            ]),
        ),
        slice(
            "web-def34",
            json!([{"addresses": ["10.244.0.31"], "conditions": {"ready": false}}]),
        ),
        backing("web-6a4f-x2k7p", "10.244.0.12", false),
        backing("web-6a4f-m9q3d", "10.244.0.13", false),
        backing("web-6a4f-t5w8r", "10.244.0.31", true),
    ] {
        ports.resources.insert(object);
    }
    let clock = Arc::new(FakeClockPort::default());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster))?;

    let target = ResourceRef::namespaced(
        cluster,
        oxikube_domain::ids::Gvk::new("", "v1", "Service"),
        "shop",
        "web",
    );
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |_, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        let tokens = if light {
            oxikube_ui::Tokens::light()
        } else {
            oxikube_ui::Tokens::dark()
        };
        oxikube_ui::set_tokens(cx, tokens);
        oxikube_runtime::init_deterministic(cx);
        cx.set_reduce_motion(true);
        oxikube_resources_ui::kinds::init(cx);
        ServiceBrowser::enable(cx);
        let clock: Arc<dyn ClockPort> = clock;
        let deps = DetailDeps {
            sessions,
            stores: Arc::new(ResourceStores::new(store_runtime(clock, cx))),
            columns: Arc::new(CoreColumns::new()),
            dispatcher: Rc::new(Ignore),
            exec: None,
            scale: None,
            set_image: None,
            drain: None,
            rollback: None,
            forward: None,
        };
        cx.new(|cx| DetailView::new(target, deps, Mount::Drawer, cx))
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |view, _, cx| {
        let detail = view.downcast::<DetailView>().expect("the root view");
        let now: Timestamp = "2026-01-10T00:00:00Z".parse().expect("a timestamp");
        detail.update(cx, |detail, cx| detail.pin_now(now, cx));
    })?;
    // A frame builds the Service's panel (it subscribes once the detail has drawn); the feeds
    // land, then the next frame draws what they brought.
    for _ in 0..3 {
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
        cx.run_until_parked();
    }
    cx.capture_screenshot(window.into())
}

pub(crate) fn run() -> anyhow::Result<()> {
    check("detail_service_dark", render(false)?, WIDTH, HEIGHT)?;
    check("detail_service_light", render(true)?, WIDTH, HEIGHT)
}
