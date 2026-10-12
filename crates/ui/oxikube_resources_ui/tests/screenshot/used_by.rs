//! The Used by tab (E12-S18) of a ConfigMap's detail, in the dark and the light theme: the
//! summary and its notes, and the workloads that use it (a Deployment found through its
//! ReplicaSet, a StatefulSet, a CronJob run's Job and a Pod nobody owns).
//!
//! The cluster is a fake one; ages are not drawn on this tab, so nothing moves with the clock.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{AppContext as _, px, size};
use oxikube_app::store::ResourceStores;
use oxikube_app::{ClusterSessionManager, CoreColumns};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_ports::{ClockPort, ClusterContext, SourceId};
use oxikube_resources_ui::detail::{DetailDeps, DetailTab, DetailView, Mount};
use oxikube_resources_ui::table::store_runtime;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, screenshot::RgbaImage,
};
use serde_json::{Value, json};

use super::{Ignore, check, headless};

const WIDTH: f32 = 520.0;
const HEIGHT: f32 = 330.0;

fn object(api_version: &str, kind: &str, name: &str, owner: Option<(&str, &str)>) -> Resource {
    let mut json = json!({
        "apiVersion": api_version, "kind": kind,
        "metadata": {"name": name, "namespace": "shop", "uid": format!("uid-{name}"),
                     "resourceVersion": "3"},
    });
    if let Some((api, owner)) = owner {
        let kind = match api {
            "batch/v1" => "CronJob",
            _ => "Deployment",
        };
        json["metadata"]["ownerReferences"] = json!([{
            "apiVersion": api, "kind": kind, "name": owner,
            "uid": format!("uid-{owner}"), "controller": true,
        }]);
    }
    Resource::from_json(json).expect("a resource")
}

fn pod(name: &str, owner: Option<(&str, &str)>) -> Resource {
    let mut json = object("v1", "Pod", name, None).to_value();
    if let Some((kind, owner)) = owner {
        let api = if kind == "StatefulSet" || kind == "ReplicaSet" {
            "apps/v1"
        } else {
            "batch/v1"
        };
        json["metadata"]["ownerReferences"] = json!([{
            "apiVersion": api, "kind": kind, "name": owner,
            "uid": format!("uid-{owner}"), "controller": true,
        }]);
    }
    json["spec"] = json!({"volumes": [{"name": "cfg", "configMap": {"name": "app-config"}}]});
    json["status"] = json!({"phase": "Running"});
    Resource::from_json(json).expect("a pod")
}

fn objects() -> Vec<Resource> {
    let config_map: Value = json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "app-config", "namespace": "shop", "resourceVersion": "3"},
        "data": {"app.yaml": "name: shop\n"},
    });
    vec![
        Resource::from_json(config_map).expect("a config map"),
        object("apps/v1", "Deployment", "web", None),
        object("apps/v1", "ReplicaSet", "web-5d", Some(("apps/v1", "web"))),
        object("apps/v1", "StatefulSet", "db", None),
        object("batch/v1", "CronJob", "nightly", None),
        object(
            "batch/v1",
            "Job",
            "nightly-2901",
            Some(("batch/v1", "nightly")),
        ),
        pod("web-5d-a", Some(("ReplicaSet", "web-5d"))),
        pod("web-5d-b", Some(("ReplicaSet", "web-5d"))),
        pod("db-0", Some(("StatefulSet", "db"))),
        pod("nightly-2901-x", Some(("Job", "nightly-2901"))),
        pod("debug-shell", None),
    ]
}

fn render(light: bool) -> anyhow::Result<RgbaImage> {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let ports = connector.ports_for(&cluster);
    for object in objects() {
        ports.resources.insert(object);
    }
    let clock = Arc::new(FakeClockPort::default());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster)).expect("connect");
    let target = ResourceRef::namespaced(
        cluster,
        Gvk::new("", "v1", "ConfigMap"),
        "shop",
        "app-config",
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
        detail.update(cx, |detail, cx| detail.set_tab(DetailTab::UsedBy, cx));
    })?;
    // The Pods land, the tab sees the index change on its next look, and the read fills the rows.
    for _ in 0..4 {
        cx.run_until_parked();
        cx.advance_clock(Duration::from_millis(300));
    }
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

pub(crate) fn run() -> anyhow::Result<()> {
    check("detail_used_by_dark", render(false)?, WIDTH, HEIGHT)?;
    check("detail_used_by_light", render(true)?, WIDTH, HEIGHT)
}
