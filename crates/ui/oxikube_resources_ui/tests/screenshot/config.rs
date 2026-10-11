//! The detail drawer of Config kinds (E12-S12), in the dark and the light theme: a ResourceQuota
//! with its used-against-hard bars (a healthy line, one near its limit and one full), and a
//! mutating webhook configuration with the `caBundle` shown as present or absent.
//!
//! Ages read against a fixed "now", so the pictures do not change with the clock.

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
use oxikube_resources_ui::table::store_runtime;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, fixtures, screenshot::RgbaImage,
};

use super::{Ignore, check, headless};

const WIDTH: f32 = 560.0;
const HEIGHT: f32 = 640.0;

fn render(object: Resource, light: bool) -> anyhow::Result<RgbaImage> {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let ports = connector.ports_for(&cluster);
    let target = ResourceRef::new(
        cluster.clone(),
        object.kind.clone(),
        object.meta.namespace.clone(),
        object.meta.name.clone(),
    );
    ports.resources.insert(object);
    let clock = Arc::new(FakeClockPort::default());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster))?;

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
        oxikube_resources_ui::kinds::init(cx);
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
        detail.update(cx, |detail, cx| detail.pin_now(now, cx));
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

/// The fixture quota with its cpu line over 90 % and its pods full.
fn quota() -> Resource {
    let mut json = fixtures::resource_quota().to_value();
    json["status"]["used"]["requests.cpu"] = serde_json::json!("3700m");
    json["status"]["used"]["pods"] = serde_json::json!("10");
    Resource::from_json(json).expect("a quota")
}

pub(crate) fn run() -> anyhow::Result<()> {
    check("detail_quota_dark", render(quota(), false)?, WIDTH, HEIGHT)?;
    check("detail_quota_light", render(quota(), true)?, WIDTH, HEIGHT)?;
    check(
        "detail_webhook_dark",
        render(fixtures::mutating_webhook(), false)?,
        WIDTH,
        HEIGHT,
    )
}
