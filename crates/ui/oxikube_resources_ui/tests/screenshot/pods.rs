//! The Pod kind's own views (E12-S02), with the Pod spec registered as the app registers it:
//!
//! - `pods_columns_dark`: the pods table with the pod columns (Ready, Status, Restarts, CPU,
//!   Memory, the four `%` columns, Service Account, Controlled By) and a fixed metrics source;
//!   one pod without limits reads `–` in the `/L` columns. Age and Restarts are hidden through a
//!   saved layout (they move with the clock).
//! - `detail_pod_containers_dark`, `detail_pod_containers_light`: a pod's detail opening on its
//!   containers sub-table: an init container, a sidecar, two regular containers (one
//!   crash-looping) and a debug container, with image, state, restarts, probes, ports and
//!   requests/limits.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, px, size};
use jiff::Timestamp;
use oxikube_app::columns::{Cell, Metric, MetricsSource};
use oxikube_app::store::ResourceStores;
use oxikube_app::{ClusterSessionManager, CoreColumns};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::{Quantity, Resource};
use oxikube_ports::{ClockPort, ClusterContext, SourceId, StatePort as _};
use oxikube_resources_ui::detail::{DetailDeps, DetailTab, DetailView, Mount};
use oxikube_resources_ui::kinds;
use oxikube_resources_ui::table::{
    ColumnPrefs, ResourceTable, ResourceTableDeps, prefs_key, store_runtime,
};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort, pod,
    screenshot::RgbaImage,
};
use serde_json::json;

use super::{Ignore, check, headless, pods_kind};

const TABLE_WIDTH: f32 = 1440.0;
const TABLE_HEIGHT: f32 = 240.0;
const DETAIL_WIDTH: f32 = 900.0;
const DETAIL_HEIGHT: f32 = 420.0;

/// CPU and memory for every pod, by name.
struct Fixed;

impl MetricsSource for Fixed {
    fn cell(&self, pod: &Resource, metric: Metric, _: Timestamp) -> Option<Cell<'static>> {
        let (cpu, memory) = match pod.meta.name.as_ref() {
            "api-7c9d" => ("180m", "230Mi"),
            "payments-9e1c" => ("12m", "40Mi"),
            "seed-db-1a3e" => return None,
            _ => ("45m", "96Mi"),
        };
        let text = match metric {
            Metric::Cpu => cpu,
            Metric::Memory => memory,
        };
        Some(Cell::quantity(text, Quantity::parse(text).ok()?))
    }
}

fn cluster() -> (ClusterId, ClusterContext) {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    (cluster, entry)
}

fn connected(objects: impl IntoIterator<Item = Resource>) -> (ClusterSessionManager, ClusterId) {
    let (cluster, entry) = cluster();
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let ports = connector.ports_for(&cluster);
    for object in objects {
        ports.resources.insert(object);
    }
    let clock = Arc::new(FakeClockPort::default());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions = ClusterSessionManager::new(connector, source, clock);
    futures::executor::block_on(sessions.connect(&cluster)).expect("connect");
    (sessions, cluster)
}

fn table_pods() -> Vec<Resource> {
    let in_ns = |name: &str| pod().namespace("shop").name(name).node("kind-worker");
    vec![
        in_ns("api-7c9d")
            .limits("500m", "256Mi")
            .controlled_by("ReplicaSet", "api-7c9d5f")
            .service_account("api")
            .build(),
        in_ns("cart-5f6b")
            .pending()
            .controlled_by("ReplicaSet", "cart-5f6b8d")
            .build(),
        in_ns("payments-9e1c")
            .crash_loop()
            .limits("200m", "128Mi")
            .controlled_by("StatefulSet", "payments")
            .service_account("payments")
            .build(),
        in_ns("seed-db-1a3e")
            .succeeded()
            .controlled_by("Job", "seed-db")
            .build(),
        in_ns("web-6a4f").terminating().build(),
    ]
}

fn render_table() -> anyhow::Result<RgbaImage> {
    let (sessions, cluster) = connected(table_pods());
    let state = Arc::new(FakeStatePort::new());
    let layout = ColumnPrefs {
        visible: [("age".to_owned(), false), ("restarts".to_owned(), false)].into(),
        ..ColumnPrefs::default()
    };
    futures::executor::block_on(state.kv_set(
        &prefs_key(&pods_kind().gvk).expect("key"),
        serde_json::to_value(layout).expect("prefs json"),
    ))?;
    let clock = Arc::new(FakeClockPort::default());
    let mut cx = headless();
    let window = cx.open_window(size(px(TABLE_WIDTH), px(TABLE_HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        oxikube_runtime::init_deterministic(cx);
        cx.set_reduce_motion(true);
        kinds::pods::init(cx);
        kinds::pods::set_metrics_source(cx, Some(Arc::new(Fixed)));
        let clock: Arc<dyn ClockPort> = clock;
        let deps = ResourceTableDeps {
            sessions,
            stores: Arc::new(ResourceStores::new(store_runtime(clock, cx))),
            columns: Arc::new(CoreColumns::new()),
            state,
            dispatcher: Rc::new(Ignore),
            actions: None,
        };
        cx.new(|cx| ResourceTable::new(cluster, pods_kind(), deps, window, cx))
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

/// `shop/api-0`: a finished init container, a started sidecar, two regular containers (one
/// crash-looping) and a debug container.
fn multi_container_pod() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "api-0", "namespace": "shop", "resourceVersion": "3",
                     "creationTimestamp": "2026-01-01T00:00:00Z"},
        "spec": {
            "initContainers": [
                {"name": "migrate", "image": "registry.example/migrate:2"},
                {"name": "proxy", "image": "envoyproxy/envoy:v1.31", "restartPolicy": "Always",
                 "startupProbe": {"tcpSocket": {"port": 15000}},
                 "ports": [{"name": "admin", "containerPort": 15000}],
                 "resources": {"requests": {"cpu": "50m", "memory": "64Mi"},
                               "limits": {"cpu": "200m", "memory": "128Mi"}}}
            ],
            "containers": [
                {"name": "api", "image": "registry.example/api:1.4.2",
                 "livenessProbe": {"httpGet": {"path": "/healthz", "port": 8080}},
                 "readinessProbe": {"httpGet": {"path": "/ready", "port": 8080}},
                 "ports": [{"name": "http", "containerPort": 8080, "protocol": "TCP"}],
                 "resources": {"requests": {"cpu": "250m", "memory": "256Mi"},
                               "limits": {"memory": "512Mi"}}},
                {"name": "worker", "image": "registry.example/worker:1.4.2",
                 "resources": {"requests": {"cpu": "0.5"}}}
            ],
            "ephemeralContainers": [{"name": "debugger", "image": "busybox:1.36"}]
        },
        "status": {
            "phase": "Running",
            "conditions": [{"type": "Initialized", "status": "True",
                            "lastTransitionTime": "2026-01-01T00:00:05Z"}],
            "initContainerStatuses": [
                {"name": "migrate", "ready": false, "restartCount": 0,
                 "state": {"terminated": {"exitCode": 0, "reason": "Completed"}}},
                {"name": "proxy", "ready": true, "started": true, "restartCount": 0,
                 "state": {"running": {"startedAt": "2026-01-01T00:00:05Z"}}}
            ],
            "containerStatuses": [
                {"name": "api", "ready": true, "started": true, "restartCount": 1,
                 "state": {"running": {"startedAt": "2026-01-01T00:00:06Z"}}},
                {"name": "worker", "ready": false, "started": false, "restartCount": 7,
                 "state": {"waiting": {"reason": "CrashLoopBackOff"}}}
            ],
            "ephemeralContainerStatuses": [
                {"name": "debugger", "ready": false, "restartCount": 0,
                 "state": {"running": {"startedAt": "2026-01-01T00:10:00Z"}}}
            ]
        }
    }))
    .expect("a pod")
}

fn render_detail(light: bool) -> anyhow::Result<RgbaImage> {
    let (sessions, cluster) = connected([multi_container_pod()]);
    let target = ResourceRef::namespaced(cluster, Gvk::new("", "v1", "Pod"), "shop", "api-0");
    let clock = Arc::new(FakeClockPort::default());
    let mut cx = headless();
    let window = cx.open_window(size(px(DETAIL_WIDTH), px(DETAIL_HEIGHT)), |_, cx| {
        oxikube_ui::init(cx);
        let tokens = if light {
            oxikube_ui::Tokens::light()
        } else {
            oxikube_ui::Tokens::dark()
        };
        oxikube_ui::set_tokens(cx, tokens);
        oxikube_runtime::init_deterministic(cx);
        cx.set_reduce_motion(true);
        kinds::pods::init(cx);
        let clock: Arc<dyn ClockPort> = clock;
        let deps = DetailDeps {
            sessions,
            stores: Arc::new(ResourceStores::new(store_runtime(clock, cx))),
            columns: Arc::new(CoreColumns::new()),
            dispatcher: Rc::new(Ignore),
            exec: None,
        };
        cx.new(|cx| DetailView::new(target, deps, Mount::Drawer, cx))
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |view, _, cx| {
        let detail = view.downcast::<DetailView>().expect("the root view");
        let now: Timestamp = "2026-01-10T00:00:00Z".parse().expect("a timestamp");
        detail.update(cx, |detail, cx| {
            detail.pin_now(now, cx);
            detail.set_tab(DetailTab::Overview, cx);
        });
    })?;
    // The first frame builds the panel; its rows land right after; the next frame draws them.
    for _ in 0..2 {
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    }
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

pub(crate) fn run() -> anyhow::Result<()> {
    check(
        "pods_columns_dark",
        render_table()?,
        TABLE_WIDTH,
        TABLE_HEIGHT,
    )?;
    check(
        "detail_pod_containers_dark",
        render_detail(false)?,
        DETAIL_WIDTH,
        DETAIL_HEIGHT,
    )?;
    check(
        "detail_pod_containers_light",
        render_detail(true)?,
        DETAIL_WIDTH,
        DETAIL_HEIGHT,
    )
}
