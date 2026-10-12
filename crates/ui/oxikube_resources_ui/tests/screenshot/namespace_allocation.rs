//! The detail drawer of a namespace, dark and light (E13-S11): the namespace panel's allocation
//! section. `shop` holds five pods that set requests, limits or neither, a ResourceQuota that
//! caps requests, limits and the pod count (one resource past 70 %, one past 90 %), and
//! metrics-server's usage for its pods. `detail_namespace_unavailable_dark` is the same namespace
//! without metrics-server (the usage line says why) and without a quota (totals, no bars).
//!
//! The ages are read against a fixed "now", and the pods and quotas are read from the store a
//! few timer ticks after the first frame, so the picture does not change with the clock.

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
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, pod, screenshot::RgbaImage,
};
use serde_json::json;

use super::{Ignore, check, headless};

const WIDTH: f32 = 480.0;
const HEIGHT: f32 = 900.0;

fn namespace() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "Namespace",
        "metadata": {
            "name": "shop",
            "resourceVersion": "7",
            "creationTimestamp": "2026-01-01T00:00:00Z",
        },
        "spec": {"finalizers": ["kubernetes"]},
        "status": {"phase": "Active"},
    }))
    .expect("a namespace")
}

fn quota() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "ResourceQuota",
        "metadata": {"name": "compute", "namespace": "shop", "resourceVersion": "3"},
        "spec": {"hard": {"requests.cpu": "2", "requests.memory": "4Gi", "pods": "6"}},
        "status": {
            "hard": {"requests.cpu": "2", "requests.memory": "4Gi", "pods": "6"},
            "used": {"requests.cpu": "1630m", "requests.memory": "2304Mi", "pods": "5"},
        },
    }))
    .expect("a quota")
}

fn in_shop(name: &str, requests: (&str, &str), limits: Option<(&str, &str)>) -> Resource {
    let mut builder = pod()
        .namespace("shop")
        .name(name)
        .node("kind-worker")
        .running()
        .requests(requests.0, requests.1);
    if let Some(limits) = limits {
        builder = builder.limits(limits.0, limits.1);
    }
    builder.build()
}

fn usage() -> MetricsOutcome {
    let ts: Timestamp = "2026-01-10T00:00:00Z".parse().expect("a timestamp");
    let sample = |name: &str, cpu: u64, memory: u64| {
        let pod = ResourceRef::namespaced(
            ClusterId::new("/home/me/.kube/config", &ContextName::new("kind-oxikube")),
            Gvk::new("", "v1", "Pod"),
            "shop",
            name,
        );
        MetricsSample::new(MetricsSubject::Pod { pod }, ts, None, cpu, memory)
    };
    MetricsOutcome::Available(vec![
        sample("api-7c9d", 410_000_000, 700 << 20),
        sample("cart-5f6b", 220_000_000, 410 << 20),
        sample("web-6a4f", 90_000_000, 180 << 20),
        sample("worker-3c1d", 60_000_000, 120 << 20),
        sample("batch-1e5f", 0, 12 << 20),
    ])
}

fn render(light: bool, metrics: MetricsOutcome, with_quota: bool) -> anyhow::Result<RgbaImage> {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let ports = connector.ports_for(&cluster);
    ports.metrics.script().pod_metrics.push_ok(metrics);
    let mut objects = vec![
        namespace(),
        in_shop("api-7c9d", ("600m", "1Gi"), Some(("2", "2Gi"))),
        in_shop("cart-5f6b", ("500m", "512Mi"), Some(("1500m", "1Gi"))),
        in_shop("web-6a4f", ("250m", "256Mi"), Some(("2", "1Gi"))),
        in_shop("worker-3c1d", ("280m", "512Mi"), None),
    ];
    // A pod that sets nothing: the "set no request" count.
    let mut bare = pod()
        .namespace("shop")
        .name("batch-1e5f")
        .node("kind-worker")
        .running()
        .json();
    bare["spec"]["containers"][0]["resources"] = json!({});
    objects.push(Resource::from_json(bare).expect("a pod"));
    if with_quota {
        objects.push(quota());
    }
    for object in objects {
        ports.resources.insert(object);
    }
    let clock = Arc::new(FakeClockPort::default());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster))?;

    let target = ResourceRef::cluster_scoped(cluster, Gvk::new("", "v1", "Namespace"), "shop");
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
            set_image: None,
            rollback: None,
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
    // The first frame builds the panel; the feeds list; the panel's tick reads the pods and
    // quotas and sums them on the background executor while the usage poll asks the metrics
    // port; the next frame draws the bars.
    for _ in 0..4 {
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
    check(
        "detail_namespace_dark",
        render(false, usage(), true)?,
        WIDTH,
        HEIGHT,
    )?;
    check(
        "detail_namespace_light",
        render(true, usage(), true)?,
        WIDTH,
        HEIGHT,
    )?;
    let absent = MetricsOutcome::Unavailable(MissingReason::NotInstalled);
    check(
        "detail_namespace_unavailable_dark",
        render(false, absent, false)?,
        WIDTH,
        HEIGHT,
    )
}
