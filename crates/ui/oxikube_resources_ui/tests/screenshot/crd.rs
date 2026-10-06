//! The Schema tab of a CRD's detail, in the dark and the light theme (E07-S07).
//!
//! The Widget CRD with `spec` and `spec.containers` opened: required markers, types as
//! `kubectl explain` writes them, enum values, a default, the version chips and the summary line.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, px, size};
use oxikube_app::store::ResourceStores;
use oxikube_app::{ClusterSessionManager, CoreColumns};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName, ResourceRef};
use oxikube_ports::{ClockPort, ClusterContext, SourceId};
use oxikube_resources_ui::crds::crd_gvk;
use oxikube_resources_ui::detail::{DetailDeps, DetailTab, DetailView, Mount};
use oxikube_resources_ui::table::store_runtime;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, screenshot::RgbaImage,
};
use serde_json::json;

use super::{Ignore, check, headless};

const WIDTH: f32 = 560.0;
const HEIGHT: f32 = 640.0;

fn widget_crd() -> Resource {
    let schema = json!({
        "type": "object",
        "required": ["spec"],
        "properties": {
            "apiVersion": {"type": "string"},
            "kind": {"type": "string"},
            "metadata": {"type": "object"},
            "spec": {
                "type": "object",
                "description": "WidgetSpec is what you want the widget to be.",
                "required": ["size", "containers"],
                "properties": {
                    "size": {
                        "type": "string",
                        "description": "How big the widget is.",
                        "enum": ["small", "medium", "large"],
                        "default": "small"
                    },
                    "replicas": {"type": "integer", "format": "int32", "default": 1},
                    "timeout": {"x-kubernetes-int-or-string": true},
                    "labels": {"type": "object", "additionalProperties": {"type": "string"}},
                    "containers": {
                        "type": "array",
                        "description": "Containers of the widget.",
                        "items": {
                            "type": "object",
                            "required": ["image", "name"],
                            "properties": {
                                "name": {"type": "string", "description": "A DNS label."},
                                "image": {"type": "string"},
                                "args": {"type": "array", "items": {"type": "string"}}
                            }
                        }
                    }
                }
            },
            "status": {"type": "object", "properties": {"phase": {"type": "string"}}}
        }
    });
    Resource::from_json(json!({
        "apiVersion": "apiextensions.k8s.io/v1",
        "kind": "CustomResourceDefinition",
        "metadata": {
            "name": "widgets.example.com",
            "resourceVersion": "5",
            "creationTimestamp": "2026-01-01T00:00:00Z"
        },
        "spec": {
            "group": "example.com",
            "scope": "Namespaced",
            "names": {
                "plural": "widgets", "singular": "widget", "kind": "Widget",
                "shortNames": ["wd"], "categories": ["all"]
            },
            "versions": [
                {"name": "v1beta1", "served": true, "storage": false, "deprecated": true,
                 "schema": {"openAPIV3Schema": {"type": "object"}}},
                {"name": "v1", "served": true, "storage": true,
                 "schema": {"openAPIV3Schema": schema}}
            ]
        }
    }))
    .expect("a CRD")
}

fn render(light: bool) -> anyhow::Result<RgbaImage> {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    connector.ports_for(&cluster).resources.insert(widget_crd());
    let clock = Arc::new(FakeClockPort::default());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster))?;

    let target = ResourceRef::cluster_scoped(cluster, crd_gvk(), "widgets.example.com");
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |_, cx| {
        oxikube_ui::init(cx);
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
        // The age is read against a fixed "now" so the picture does not change with the clock.
        let now: jiff::Timestamp = "2026-01-10T00:00:00Z".parse().expect("a timestamp");
        detail.update(cx, |detail, cx| {
            detail.pin_now(now, cx);
            detail.set_tab(DetailTab::Schema, cx);
            detail.toggle_schema("spec", cx);
            detail.toggle_schema("spec.containers", cx);
        });
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

pub(crate) fn run() -> anyhow::Result<()> {
    check("detail_crd_schema_dark", render(false)?, WIDTH, HEIGHT)?;
    check("detail_crd_schema_light", render(true)?, WIDTH, HEIGHT)
}
