//! The Pod kind's own views (E12-S02), with the Pod spec registered as the app registers it:
//!
//! - `pods_columns_dark`: the pods table with the pod columns (Ready, Status, Restarts, CPU,
//!   Memory, the four `%` columns, Service Account, Controlled By) over the app's metrics
//!   service fed one poll (E13-S03): the API pod at 90 % of its memory limit (red, k9s'
//!   critical), the payments pod at 75 % of its CPU limit (amber); pods without limits read `–`
//!   in the `/L` columns, the pending and the succeeded ones have no sample. Age and Restarts are hidden through a
//!   saved layout (they move with the clock).
//! - `pods_metrics_unavailable_dark`: the same table on a cluster without metrics-server: the
//!   `metrics unavailable: <reason>` badge in the toolbar and `–` in every usage cell.
//! - `detail_pod_containers_dark`, `detail_pod_containers_light`: a pod's detail opening on its
//!   containers sub-table: an init container, a sidecar, two regular containers (one
//!   crash-looping) and a debug container, with image, state, restarts, probes, ports and
//!   requests/limits.
//! - `detail_pod_panel_dark`, `detail_pod_panel_light` (E12-S03): the pod panel of a pod with an
//!   owner chain (ReplicaSet, Deployment), a node and an opened Environment: a ConfigMap value, a
//!   Secret value masked, a missing reference as a chip, a field reference resolved.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, px, size};
use jiff::Timestamp;
use oxikube_app::store::ResourceStores;
use oxikube_app::{ClusterSessionManager, CoreColumns, MetricsKind, MetricsService};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::metrics::{
    ContainerUsage, MetricsSample, MetricsSubject, MissingReason, Reading,
};
use oxikube_ports::MetricsOutcome;
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
/// Wide enough for every column of the containers sub-table (it scrolls sideways in a drawer).
const DETAIL_WIDTH: f32 = 1640.0;
const DETAIL_HEIGHT: f32 = 420.0;
/// Tall enough for the action bar, the containers, the open sections and the Overview below.
const PANEL_HEIGHT: f32 = 700.0;
const PANEL_WIDTH: f32 = 900.0;

/// One poll of metrics-server, by pod name: `(millicores, MiB)`; the pending and the succeeded
/// pods have no sample (metrics-server only reports running containers).
fn table_reading(cluster: &ClusterId) -> MetricsOutcome {
    let reading = |name: &str, millicores: u64, mib: u64| {
        let pod = ResourceRef::namespaced(cluster.clone(), Gvk::new("", "v1", "Pod"), "shop", name);
        MetricsSample::new(
            MetricsSubject::Pod { pod },
            Timestamp::UNIX_EPOCH,
            None,
            millicores * 1_000_000,
            mib * 1024 * 1024,
        )
    };
    MetricsOutcome::Available(vec![
        reading("api-7c9d", 180, 230),
        reading("payments-9e1c", 150, 40),
        reading("web-6a4f", 45, 96),
    ])
}

/// The app's metrics service over `sessions`, with `outcome` as the last pod poll of `cluster`.
fn metrics(
    sessions: &ClusterSessionManager,
    cluster: &ClusterId,
    outcome: MetricsOutcome,
) -> MetricsService {
    let service = MetricsService::new(sessions.clone(), Arc::new(FakeClockPort::default()));
    service
        .usage(cluster)
        .apply(MetricsKind::Pods, Timestamp::UNIX_EPOCH, &Ok(outcome));
    service
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

fn render_table(available: bool) -> anyhow::Result<RgbaImage> {
    let (sessions, cluster) = connected(table_pods());
    let outcome = if available {
        table_reading(&cluster)
    } else {
        MetricsOutcome::Unavailable(MissingReason::NotInstalled)
    };
    let service = metrics(&sessions, &cluster, outcome);
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
        oxikube_resources_ui::metrics::set_service(cx, service);
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

/// `shop/api-0`'s containers: the sidecar, the API (90 % of its memory limit) and the worker.
fn detail_reading(target: &ResourceRef) -> MetricsOutcome {
    let container = |name: &str, millicores: u64, mib: u64| ContainerUsage {
        name: name.into(),
        cpu: Reading::Value(millicores * 1_000_000),
        memory: Reading::Value(mib * 1024 * 1024),
    };
    let mut sample = MetricsSample::new(
        MetricsSubject::Pod {
            pod: target.clone(),
        },
        Timestamp::UNIX_EPOCH,
        None,
        203_000_000,
        513 * 1024 * 1024,
    );
    sample.containers = vec![
        container("proxy", 20, 40),
        container("api", 180, 461),
        container("worker", 3, 12),
    ];
    MetricsOutcome::Available(vec![sample])
}

fn render_detail(light: bool) -> anyhow::Result<RgbaImage> {
    let (sessions, cluster) = connected([multi_container_pod()]);
    let target = ResourceRef::namespaced(cluster, Gvk::new("", "v1", "Pod"), "shop", "api-0");
    let service = metrics(&sessions, &target.cluster, detail_reading(&target));
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
        oxikube_resources_ui::metrics::set_service(cx, service);
        let clock: Arc<dyn ClockPort> = clock;
        let deps = DetailDeps {
            sessions,
            stores: Arc::new(ResourceStores::new(store_runtime(clock, cx))),
            columns: Arc::new(CoreColumns::new()),
            dispatcher: Rc::new(Ignore),
            exec: None,
            scale: None,
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

/// `shop/web-0`: owned by the ReplicaSet `web-5d` (owned by the Deployment `web`), on a node, with
/// an environment that names a ConfigMap, a Secret and a ConfigMap that is not there.
fn env_pod() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "web-0", "namespace": "shop", "resourceVersion": "9",
                     "creationTimestamp": "2026-01-01T00:00:00Z",
                     "ownerReferences": [{"apiVersion": "apps/v1", "kind": "ReplicaSet",
                                          "name": "web-5d", "uid": "rs", "controller": true}]},
        "spec": {
            "nodeName": "kind-worker",
            "containers": [{
                "name": "web", "image": "registry.example/web:2.1",
                "envFrom": [{"configMapRef": {"name": "web-config"}}],
                "env": [
                    {"name": "DATABASE_PASSWORD", "valueFrom": {"secretKeyRef": {"name": "web-db", "key": "password"}}},
                    {"name": "FEATURE_FLAGS", "valueFrom": {"configMapKeyRef": {"name": "flags", "key": "all"}}},
                    {"name": "POD_IP", "valueFrom": {"fieldRef": {"fieldPath": "status.podIP"}}},
                    {"name": "LOG_LEVEL", "value": "info"}
                ],
                "volumeMounts": [{"name": "config", "mountPath": "/etc/web", "readOnly": true}],
                "livenessProbe": {"httpGet": {"path": "/healthz", "port": 8080}}
            }],
            "volumes": [{"name": "config", "configMap": {"name": "web-config"}}]
        },
        "status": {"phase": "Running", "podIP": "10.244.1.17",
                   "containerStatuses": [{"name": "web", "ready": true, "restartCount": 0,
                                          "state": {"running": {"startedAt": "2026-01-01T00:00:06Z"}}}]}
    }))
    .expect("a pod")
}

fn env_objects() -> Vec<Resource> {
    let from = |value: serde_json::Value| Resource::from_json(value).expect("an object");
    vec![
        env_pod(),
        from(json!({"apiVersion": "v1", "kind": "ConfigMap",
                    "metadata": {"name": "web-config", "namespace": "shop"},
                    "data": {"REGION": "eu-west-1", "WORKERS": "4"}})),
        from(json!({"apiVersion": "v1", "kind": "Secret",
                    "metadata": {"name": "web-db", "namespace": "shop"},
                    "data": {"password": "aHVudGVyMg=="}})),
        from(json!({"apiVersion": "apps/v1", "kind": "ReplicaSet",
                    "metadata": {"name": "web-5d", "namespace": "shop", "ownerReferences": [
                        {"apiVersion": "apps/v1", "kind": "Deployment", "name": "web",
                         "uid": "dep", "controller": true}]}})),
        from(json!({"apiVersion": "apps/v1", "kind": "Deployment",
                    "metadata": {"name": "web", "namespace": "shop"}})),
    ]
}

fn render_panel(light: bool) -> anyhow::Result<RgbaImage> {
    let (sessions, cluster) = connected(env_objects());
    let target = ResourceRef::namespaced(cluster, Gvk::new("", "v1", "Pod"), "shop", "web-0");
    let clock = Arc::new(FakeClockPort::default());
    let mut cx = headless();
    let window = cx.open_window(size(px(PANEL_WIDTH), px(PANEL_HEIGHT)), |_, cx| {
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
            scale: None,
            set_image: None,
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
    for _ in 0..2 {
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    }
    // Open the environment and the mounts; the ConfigMaps and the Secret are read right after.
    cx.update_window(window.into(), |view, _, cx| {
        let detail = view.downcast::<DetailView>().expect("the root view");
        let panel = detail
            .read(cx)
            .kind_panel()
            .cloned()
            .expect("the pod's panel")
            .downcast::<kinds::pods::PodPanel>()
            .expect("the pod panel");
        panel.update(cx, |panel, cx| {
            panel.toggle(kinds::pods::panel::SectionId::Environment, cx);
            panel.toggle(kinds::pods::panel::SectionId::Mounts, cx);
        });
    })?;
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
        render_table(true)?,
        TABLE_WIDTH,
        TABLE_HEIGHT,
    )?;
    check(
        "pods_metrics_unavailable_dark",
        render_table(false)?,
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
    )?;
    check(
        "detail_pod_panel_dark",
        render_panel(false)?,
        PANEL_WIDTH,
        PANEL_HEIGHT,
    )?;
    check(
        "detail_pod_panel_light",
        render_panel(true)?,
        PANEL_WIDTH,
        PANEL_HEIGHT,
    )
}
