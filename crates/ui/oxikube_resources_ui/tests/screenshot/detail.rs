//! The detail drawer of a Deployment, in the dark and the light theme: the Overview, then the YAML
//! tab (read-only, highlighted) and the Describe tab (E07-S06).
//!
//! The Deployment has labels, an annotation, conditions and a status; the age is read against a
//! fixed "now" so the picture does not change with the clock.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, px, size};
use jiff::Timestamp;
use oxikube_app::store::ResourceStores;
use oxikube_app::{ClusterSessionManager, CoreColumns};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName, ResourceRef};
use oxikube_ports::{ClockPort, ClusterContext, SourceId};
use oxikube_ports::{DescribeOutput, DescribeSource};
use oxikube_resources_ui::detail::{DetailDeps, DetailTab, DetailView, Mount};
use oxikube_resources_ui::table::store_runtime;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, deployment, pod,
    screenshot::RgbaImage,
};
use serde_json::json;

use super::{Ignore, check, headless};

const WIDTH: f32 = 480.0;
const HEIGHT: f32 = 760.0;

fn web_deployment() -> Resource {
    let mut json = deployment()
        .namespace("shop")
        .name("web")
        .label("app", "web")
        .label("tier", "frontend")
        .annotation("deployment.kubernetes.io/revision", "7")
        .created("2026-01-01T00:00:00Z")
        .replicas(3)
        .ready(3)
        .updated(3)
        .available(3)
        .json();
    json["metadata"]["resourceVersion"] = json!("42");
    json["metadata"]["finalizers"] = json!(["example.com/cleanup"]);
    json["status"]["conditions"] = json!([
        {"type": "Available", "status": "True", "reason": "MinimumReplicasAvailable",
         "message": "Deployment has minimum availability.",
         "lastTransitionTime": "2026-01-01T00:01:00Z"},
        {"type": "Progressing", "status": "True", "reason": "NewReplicaSetAvailable",
         "message": "ReplicaSet \"web-5d8c7\" has successfully progressed.",
         "lastTransitionTime": "2026-01-02T00:00:00Z"}
    ]);
    Resource::from_json(json).expect("a deployment")
}

/// A pod with the audit's problem cases: a condition type too long for a column, a long
/// condition message, and a label and an annotation key longer than the key column.
fn crashing_pod() -> Resource {
    let mut json = pod()
        .namespace("shop")
        .name("web-imagepull")
        .label("app", "web")
        .label("app.kubernetes.io/a-very-long-label-key-name", "frontend")
        .annotation(
            "kubectl.kubernetes.io/last-applied-configuration-key",
            "{\"apiVersion\":\"v1\"}",
        )
        .created("2026-01-01T00:00:00Z")
        .json();
    json["metadata"]["resourceVersion"] = json!("7");
    json["status"]["conditions"] = json!([
        {"type": "PodReadyToStartContainers", "status": "True",
         "lastTransitionTime": "2026-01-01T00:00:10Z"},
        {"type": "Ready", "status": "False", "reason": "ContainersNotReady",
         "message": "containers with unready status: [web sidecar] after a long wait",
         "lastTransitionTime": "2026-01-01T00:00:20Z"},
        {"type": "ContainersReady", "status": "False", "reason": "ContainersNotReady",
         "lastTransitionTime": "2026-01-01T00:00:20Z"}
    ]);
    Resource::from_json(json).expect("a pod")
}

/// What `kubectl describe deployment web` prints, as the tab shows it.
const DESCRIBE: &str = "Name:                   web\nNamespace:              shop\nLabels:                 app=web\n                        tier=frontend\nAnnotations:            deployment.kubernetes.io/revision: 7\nReplicas:               3 desired | 3 updated | 3 total | 3 available | 0 unavailable\nStrategyType:           RollingUpdate\nConditions:\n  Type           Status  Reason\n  ----           ------  ------\n  Available      True    MinimumReplicasAvailable\n  Progressing    True    NewReplicaSetAvailable\nEvents:                 <none>\n";

fn render(light: bool, tab: DetailTab) -> anyhow::Result<RgbaImage> {
    render_of(web_deployment(), light, tab)
}

fn render_of(object: Resource, light: bool, tab: DetailTab) -> anyhow::Result<RgbaImage> {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let ports = connector.ports_for(&cluster);
    let gvk = object.kind.clone();
    let name = object.meta.name.to_string();
    ports.resources.insert(object);
    ports.describe.script().describe.push_ok(DescribeOutput {
        text: DESCRIBE.to_owned(),
        source: DescribeSource::Native,
    });
    let clock = Arc::new(FakeClockPort::default());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster))?;

    let target = ResourceRef::namespaced(cluster, gvk, "shop", name);
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
            detail.set_tab(tab, cx);
        });
    })?;
    cx.run_until_parked();
    // A frame, so the editors of the YAML and Describe tabs exist; then the text lands and the
    // next frame draws it.
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

pub(crate) fn run() -> anyhow::Result<()> {
    check(
        "detail_deployment_dark",
        render(false, DetailTab::Overview)?,
        WIDTH,
        HEIGHT,
    )?;
    check(
        "detail_deployment_light",
        render(true, DetailTab::Overview)?,
        WIDTH,
        HEIGHT,
    )?;
    check(
        "detail_yaml_dark",
        render(false, DetailTab::Yaml)?,
        WIDTH,
        HEIGHT,
    )?;
    check(
        "detail_yaml_light",
        render(true, DetailTab::Yaml)?,
        WIDTH,
        HEIGHT,
    )?;
    check(
        "detail_describe_dark",
        render(false, DetailTab::Describe)?,
        WIDTH,
        HEIGHT,
    )?;
    check(
        "detail_pod_conditions_dark",
        render_of(crashing_pod(), false, DetailTab::Overview)?,
        WIDTH,
        HEIGHT,
    )?;
    check(
        "detail_pod_conditions_light",
        render_of(crashing_pod(), true, DetailTab::Overview)?,
        WIDTH,
        HEIGHT,
    )
}
