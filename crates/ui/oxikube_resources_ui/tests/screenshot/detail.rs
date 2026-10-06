//! The detail drawer of a Deployment, in the dark and the light theme.
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
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_ports::{ClockPort, ClusterContext, SourceId};
use oxikube_resources_ui::detail::{DetailDeps, DetailView, Mount};
use oxikube_resources_ui::table::store_runtime;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, deployment,
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

fn render(light: bool) -> anyhow::Result<RgbaImage> {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    connector
        .ports_for(&cluster)
        .resources
        .insert(web_deployment());
    let clock = Arc::new(FakeClockPort::default());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster))?;

    let target =
        ResourceRef::namespaced(cluster, Gvk::new("apps", "v1", "Deployment"), "shop", "web");
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
        };
        cx.new(|cx| DetailView::new(target, deps, Mount::Drawer, cx))
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |view, _, cx| {
        let detail = view.downcast::<DetailView>().expect("the root view");
        let now: Timestamp = "2026-01-10T00:00:00Z".parse().expect("a timestamp");
        detail.update(cx, |detail, cx| detail.pin_now(now, cx));
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

pub(crate) fn run() -> anyhow::Result<()> {
    check("detail_deployment_dark", render(false)?, WIDTH, HEIGHT)?;
    check("detail_deployment_light", render(true)?, WIDTH, HEIGHT)
}
