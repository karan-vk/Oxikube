//! The storage views (E12-S15), in the dark and the light theme: the claims table with its Pods
//! (used by) column, a claim's detail panel with the Pods that mount it, and the resize dialog
//! refusing a shrink.
//!
//! The cluster is a fake one: three claims (one growing, one pending), a storage class that
//! allows expansion, and Pods that mount the claims. Ages are hidden or read against a fixed
//! "now" so the pictures do not change with the clock.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{AppContext as _, WeakEntity, px, size};
use jiff::Timestamp;
use oxikube_app::command_bus::{CommandBus, CommandRegistry};
use oxikube_app::store::ResourceStores;
use oxikube_app::{ClusterSessionManager, CoreColumns, MutationGuard, ResizeFlow};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_ports::{ClockPort, ClusterContext, SourceId, StatePort as _};
use oxikube_resources_ui::detail::{DetailDeps, DetailView, Mount};
use oxikube_resources_ui::kinds::storage::dialogs::ResizeDialog;
use oxikube_resources_ui::table::{
    ColumnPrefs, ResourceTable, ResourceTableDeps, prefs_key, store_runtime,
};
use oxikube_testkit::storage::{pod_mounting, pvc, storage_class};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
    screenshot::RgbaImage,
};

use super::{Ignore, check, headless};

const WIDTH: f32 = 880.0;
const HEIGHT: f32 = 230.0;
const PANEL_WIDTH: f32 = 520.0;
const PANEL_HEIGHT: f32 = 420.0;
const DIALOG_WIDTH: f32 = 580.0;
const DIALOG_HEIGHT: f32 = 340.0;

fn claims_kind() -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new("", "v1", "PersistentVolumeClaim"),
        preferred: true,
        plural: "persistentvolumeclaims".into(),
        singular: "persistentvolumeclaim".into(),
        short_names: vec!["pvc".into()],
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch", "delete", "patch"]),
        namespaced: true,
    }
}

fn objects() -> Vec<Resource> {
    let mut data = pvc("data")
        .namespace("shop")
        .class("fast")
        .requests("20Gi")
        .capacity("10Gi")
        .condition("FileSystemResizePending", "True", "Waiting for the kubelet")
        .build();
    data.meta.resource_version = Some("11".into());
    vec![
        data,
        pvc("logs")
            .namespace("shop")
            .class("slow")
            .requests("1Gi")
            .capacity("1Gi")
            .build(),
        pvc("scratch")
            .namespace("shop")
            .class("fast")
            .pending()
            .build(),
        storage_class("fast")
            .expandable(true)
            .default_class()
            .build(),
        storage_class("slow").build(),
        pod_mounting("web-0", "shop", &["data"]),
        pod_mounting("web-1", "shop", &["data", "logs"]),
        pod_mounting("worker-0", "shop", &["logs"]),
    ]
}

struct World {
    sessions: ClusterSessionManager,
    cluster: ClusterId,
    state: Arc<FakeStatePort>,
    clock: Arc<FakeClockPort>,
}

fn world() -> World {
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
    World {
        sessions,
        cluster,
        state: Arc::new(FakeStatePort::new()),
        clock,
    }
}

fn pin(cx: &mut gpui::App, light: bool) {
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
}

fn table(light: bool) -> anyhow::Result<RgbaImage> {
    let w = world();
    // The age moves with the clock: hide it through a saved layout.
    let layout = ColumnPrefs {
        visible: [("age".to_owned(), false)].into(),
        ..ColumnPrefs::default()
    };
    futures::executor::block_on(w.state.kv_set(
        &prefs_key(&claims_kind().gvk).expect("key"),
        serde_json::to_value(layout).expect("prefs json"),
    ))
    .expect("store prefs");
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        pin(cx, light);
        oxikube_resources_ui::kinds::init(cx);
        let clock: Arc<dyn ClockPort> = w.clock;
        let deps = ResourceTableDeps {
            sessions: w.sessions,
            stores: Arc::new(ResourceStores::new(store_runtime(clock, cx))),
            columns: Arc::new(CoreColumns::new()),
            state: w.state,
            dispatcher: Rc::new(Ignore),
            actions: None,
        };
        cx.new(|cx| ResourceTable::new(w.cluster, claims_kind(), deps, window, cx))
    })?;
    cx.run_until_parked();
    // The Pods column follows the Pods feed on the table's once-a-second tick.
    cx.advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(window.into(), |view, _, cx| {
        let table = view.downcast::<ResourceTable>().expect("the root view");
        table.update(cx, |table, cx| table.move_cursor(1, false, cx));
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

fn panel(light: bool) -> anyhow::Result<RgbaImage> {
    let w = world();
    let target = ResourceRef::namespaced(w.cluster.clone(), claims_kind().gvk, "shop", "data");
    let mut cx = headless();
    let window = cx.open_window(size(px(PANEL_WIDTH), px(PANEL_HEIGHT)), |_, cx| {
        pin(cx, light);
        oxikube_resources_ui::kinds::init(cx);
        let clock: Arc<dyn ClockPort> = w.clock;
        let deps = DetailDeps {
            sessions: w.sessions,
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
        detail.update(cx, |detail, cx| detail.pin_now(now, cx));
    })?;
    cx.run_until_parked();
    // A frame builds the panel; it fills right after, and the Pods that mount the claim land
    // on its next look.
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

fn dialog(light: bool) -> anyhow::Result<RgbaImage> {
    let w = world();
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::storage", |r| {
            oxikube_app::storage::register_commands(r, w.sessions.clone())
        })
        .expect("register the storage commands");
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(w.sessions.clone(), w.state.clone(), w.clock.clone()),
    );
    let flow = ResizeFlow::new(bus, w.sessions.clone(), "alice");
    let target = ResourceRef::namespaced(w.cluster.clone(), claims_kind().gvk, "shop", "data");
    let mut cx = headless();
    let window = cx.open_window(size(px(DIALOG_WIDTH), px(DIALOG_HEIGHT)), |window, cx| {
        pin(cx, light);
        cx.new(|cx| ResizeDialog::new(flow, target, WeakEntity::new_invalid(), window, cx))
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |view, window, cx| {
        let dialog = view.downcast::<ResizeDialog>().expect("the root view");
        dialog.update(cx, |dialog, cx| dialog.type_size("5Gi", window, cx));
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

pub(crate) fn run() -> anyhow::Result<()> {
    check("storage_claims_table_dark", table(false)?, WIDTH, HEIGHT)?;
    check("storage_claims_table_light", table(true)?, WIDTH, HEIGHT)?;
    check(
        "storage_claim_panel_dark",
        panel(false)?,
        PANEL_WIDTH,
        PANEL_HEIGHT,
    )?;
    check(
        "storage_claim_panel_light",
        panel(true)?,
        PANEL_WIDTH,
        PANEL_HEIGHT,
    )?;
    check(
        "storage_resize_dialog_dark",
        dialog(false)?,
        DIALOG_WIDTH,
        DIALOG_HEIGHT,
    )?;
    check(
        "storage_resize_dialog_light",
        dialog(true)?,
        DIALOG_WIDTH,
        DIALOG_HEIGHT,
    )
}
