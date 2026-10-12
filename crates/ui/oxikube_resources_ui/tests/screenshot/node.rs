//! The detail drawer of a node, in the dark and the light theme (E12-S09, E13-S11): the node panel
//! above the generic Overview. A cordoned worker with a taint of its own, a memory-pressure
//! condition, three pods on it (CPU requests at 25 % of allocatable, limits past it), two GPUs, and
//! images; the allocation bars carry the live usage from metrics-server. A third picture
//! (`detail_node_unavailable_dark`) is the same node without metrics-server: requests and limits
//! stay, the usage line says why it is missing.
//!
//! The ages are read against a fixed "now", and the pods are listed by the store's pods-by-node
//! index (two timer ticks after the first frame), so the picture does not change with the clock.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{AppContext as _, px, size};
use jiff::Timestamp;
use oxikube_app::store::ResourceStores;
use oxikube_app::{ClusterSessionManager, CoreColumns};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::metrics::{MetricsSample, MetricsSubject, MissingReason};
use oxikube_ports::{ClockPort, ClusterContext, MetricsOutcome, SourceId};
use oxikube_resources_ui::detail::{DetailDeps, DetailView, Mount};
use oxikube_resources_ui::table::store_runtime;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, node, pod,
    screenshot::RgbaImage,
};
use serde_json::json;

use super::{Ignore, check, headless};

const WIDTH: f32 = 480.0;
const HEIGHT: f32 = 900.0;

fn worker() -> Resource {
    let mut json = node()
        .name("worker-2")
        .role("worker")
        .label("node.kubernetes.io/instance-type", "m5.xlarge")
        .label("topology.kubernetes.io/zone", "eu-west-1b")
        .created("2026-01-01T00:00:00Z")
        .cordoned()
        .condition("MemoryPressure", true)
        .taint("dedicated", "gpu", "NoSchedule")
        .allocatable("3920m", "15Gi", "110")
        .image("docker.io/library/nginx:1.29", 190 << 20)
        .image("registry.k8s.io/pause:3.10", 700 << 10)
        .image("ghcr.io/example/payments-api:2026.10.3", 412 << 20)
        .json();
    json["metadata"]["resourceVersion"] = json!("42");
    json["status"]["allocatable"]["nvidia.com/gpu"] = json!("2");
    Resource::from_json(json).expect("a node")
}

fn on_worker(name: &str, requests: (&str, &str), limits: (&str, &str)) -> Resource {
    let mut json = pod()
        .namespace("shop")
        .name(name)
        .node("worker-2")
        .running()
        .requests(requests.0, requests.1)
        .limits(limits.0, limits.1)
        .json();
    if name == "api-7c9d" {
        json["spec"]["containers"][0]["resources"]["requests"]["nvidia.com/gpu"] = json!("1");
        json["spec"]["containers"][0]["resources"]["limits"]["nvidia.com/gpu"] = json!("1");
    }
    Resource::from_json(json).expect("a pod")
}

fn usage() -> MetricsOutcome {
    let ts: Timestamp = "2026-01-10T00:00:00Z".parse().expect("a timestamp");
    MetricsOutcome::Available(vec![MetricsSample::new(
        MetricsSubject::Node {
            node: "worker-2".into(),
        },
        ts,
        None,
        1_450_000_000,
        6 << 30,
    )])
}

fn render(light: bool, metrics: MetricsOutcome) -> anyhow::Result<RgbaImage> {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let ports = connector.ports_for(&cluster);
    ports.metrics.script().node_metrics.push_ok(metrics);
    for object in [
        worker(),
        on_worker("api-7c9d", ("300m", "1Gi"), ("1", "2Gi")),
        on_worker("cart-5f6b", ("500m", "512Mi"), ("2", "1Gi")),
        on_worker("web-6a4f", ("180m", "256Mi"), ("4", "4Gi")),
    ] {
        ports.resources.insert(object);
    }
    let clock = Arc::new(FakeClockPort::default());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster))?;

    let target = ResourceRef::cluster_scoped(cluster, Gvk::new("", "v1", "Node"), "worker-2");
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
        let clock: Arc<dyn ClockPort> = clock;
        let deps = DetailDeps {
            sessions,
            stores: Arc::new(ResourceStores::new(store_runtime(clock, cx))),
            columns: Arc::new(CoreColumns::new()),
            dispatcher: Rc::new(Ignore),
            exec: None,
            scale: None,
            drain: None,
        };
        cx.new(|cx| DetailView::new(target, deps, Mount::Drawer, cx))
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |view, _, cx| {
        let detail = view.downcast::<DetailView>().expect("the root view");
        let now: Timestamp = "2026-01-10T00:00:00Z".parse().expect("a timestamp");
        detail.update(cx, |detail, cx| detail.pin_now(now, cx));
    })?;
    // The first frame builds the panel; the Pod feed lists; the panel's quarter-second tick
    // picks the pods up and the usage poll asks the metrics port; the next frame draws the bars.
    for _ in 0..3 {
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
        cx.run_until_parked();
        cx.advance_clock(Duration::from_millis(300));
        cx.run_until_parked();
    }
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

pub(crate) fn run() -> anyhow::Result<()> {
    check("detail_node_dark", render(false, usage())?, WIDTH, HEIGHT)?;
    check("detail_node_light", render(true, usage())?, WIDTH, HEIGHT)?;
    let absent = MetricsOutcome::Unavailable(MissingReason::NotInstalled);
    check(
        "detail_node_unavailable_dark",
        render(false, absent)?,
        WIDTH,
        HEIGHT,
    )
}
