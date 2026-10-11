//! The Namespaces kind (E12-S16): the table with its pod counts, the delete dialog with its
//! warnings (a namespace with HNC subnamespaces and a lot in it), and the create dialog showing
//! an invalid name.
//!
//! Ages are hidden through a saved layout so the pictures do not change with the clock.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, WeakEntity, px, size};
use oxikube_app::command_bus::{CommandBus, CommandRegistry};
use oxikube_app::namespaces::{NamespaceCreator, delete_command};
use oxikube_app::store::ResourceStores;
use oxikube_app::{ClusterSessionManager, CoreColumns, DeleteFlow, MutationGuard};
use oxikube_domain::Resource;
use oxikube_domain::command::Propagation;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_domain::view::{NamespaceSummary, delete_notes};
use oxikube_ports::{ClockPort, ClusterContext, SourceId, StatePort as _};
use oxikube_resources_ui::actions::DeleteDialog;
use oxikube_resources_ui::kinds::KindSpecs;
use oxikube_resources_ui::kinds::namespace::{CreateNamespaceDialog, spec};
use oxikube_resources_ui::table::{
    ColumnPrefs, ResourceTable, ResourceTableDeps, prefs_key, store_runtime,
};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort, pod,
    screenshot::RgbaImage,
};
use serde_json::json;

use super::{Ignore, check, headless};

const TABLE_WIDTH: f32 = 720.0;
const TABLE_HEIGHT: f32 = 200.0;
const DELETE_WIDTH: f32 = 560.0;
const DELETE_HEIGHT: f32 = 330.0;
const CREATE_WIDTH: f32 = 520.0;
const CREATE_HEIGHT: f32 = 330.0;

fn namespaces_kind() -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new("", "v1", "Namespace"),
        preferred: true,
        plural: "namespaces".into(),
        singular: "namespace".into(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: VerbSet::from_names(["list", "watch", "create", "delete"]),
        namespaced: false,
    }
}

fn namespace(name: &str, labels: serde_json::Value, phase: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "Namespace",
        "metadata": {
            "name": name,
            "resourceVersion": "1",
            "creationTimestamp": "2026-01-01T00:00:00Z",
            "labels": labels,
        },
        "status": { "phase": phase },
    }))
    .expect("a namespace")
}

fn cluster() -> (ClusterId, ClusterContext) {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    (cluster, entry)
}

fn pin_theme(cx: &mut gpui::App, light: bool) {
    oxikube_ui::init(cx);
    let tokens = if light {
        oxikube_ui::Tokens::light()
    } else {
        oxikube_ui::Tokens::dark()
    };
    oxikube_ui::set_tokens(cx, tokens);
    oxikube_runtime::init_deterministic(cx);
    cx.set_reduce_motion(true);
}

fn render_table(light: bool) -> anyhow::Result<RgbaImage> {
    let (cluster, entry) = cluster();
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let ports = connector.ports_for(&cluster);
    for resource in [
        namespace("default", json!({}), "Active"),
        namespace("kube-system", json!({}), "Active"),
        namespace("team-a", json!({ "team": "a" }), "Active"),
        namespace("team-b", json!({ "team": "b" }), "Terminating"),
    ] {
        ports.resources.insert(resource);
    }
    let in_ns = |ns: &str, name: &str| pod().namespace(ns).name(name).running().build();
    for resource in [
        in_ns("kube-system", "coredns-1"),
        in_ns("kube-system", "coredns-2"),
        in_ns("kube-system", "etcd-0"),
        in_ns("team-a", "web-0"),
        in_ns("team-a", "web-1"),
        in_ns("team-b", "db-0"),
    ] {
        ports.resources.insert(resource);
    }
    let clock = Arc::new(FakeClockPort::default());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster)).expect("connect");
    let state = Arc::new(FakeStatePort::new());
    let layout = ColumnPrefs {
        visible: [("age".to_owned(), false), ("labels".to_owned(), true)].into(),
        ..ColumnPrefs::default()
    };
    futures::executor::block_on(state.kv_set(
        &prefs_key(&namespaces_kind().gvk).expect("key"),
        serde_json::to_value(layout).expect("prefs json"),
    ))
    .expect("store prefs");

    let mut cx = headless();
    let window = cx.open_window(size(px(TABLE_WIDTH), px(TABLE_HEIGHT)), |window, cx| {
        pin_theme(cx, light);
        KindSpecs::register(cx, spec()).expect("the Namespace kind registers");
        let clock: Arc<dyn ClockPort> = clock;
        let deps = ResourceTableDeps {
            sessions,
            stores: Arc::new(ResourceStores::new(store_runtime(clock, cx))),
            columns: Arc::new(CoreColumns::new()),
            state,
            dispatcher: Rc::new(Ignore),
            actions: None,
        };
        cx.new(|cx| ResourceTable::new(cluster, namespaces_kind(), deps, window, cx))
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |view, window, cx| {
        let table = view.downcast::<ResourceTable>().expect("the root view");
        table.update(cx, |table, cx| table.move_cursor(2, false, cx));
        window.draw(cx).clear(cx);
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

fn render_delete(light: bool) -> anyhow::Result<RgbaImage> {
    let (cluster, entry) = cluster();
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new().with_contexts([entry.clone()])),
        clock.clone(),
    );
    // The dialog names the cluster by its context.
    sessions.open(&entry, Default::default());
    let bus = CommandBus::new(
        CommandRegistry::new(),
        MutationGuard::new(sessions.clone(), Arc::new(FakeStatePort::new()), clock),
    );
    let target = ResourceRef::cluster_scoped(cluster, Gvk::new("", "v1", "Namespace"), "team-a");
    let flow = DeleteFlow::new(bus, sessions, "me")
        .with_command(delete_command(std::collections::HashMap::new()));
    let plan = flow
        .plan(&[target], Propagation::Background)
        .expect("a plan");
    let notes = delete_notes(
        &NamespaceSummary::named("team-a"),
        2,
        &[
            ("Pod".to_owned(), 12),
            ("Deployment".to_owned(), 3),
            ("Service".to_owned(), 4),
        ],
    );
    let mut cx = headless();
    let window = cx.open_window(size(px(DELETE_WIDTH), px(DELETE_HEIGHT)), |window, cx| {
        pin_theme(cx, light);
        cx.new(|cx| {
            DeleteDialog::new(flow, plan, WeakEntity::new_invalid(), window, cx).with_notes(notes)
        })
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

fn render_create() -> anyhow::Result<RgbaImage> {
    let (cluster, entry) = cluster();
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new().with_contexts([entry])),
        clock.clone(),
    );
    let bus = CommandBus::new(
        CommandRegistry::new(),
        MutationGuard::new(sessions, Arc::new(FakeStatePort::new()), clock),
    );
    let creator = NamespaceCreator::new(bus, "me");
    let mut cx = headless();
    let window = cx.open_window(size(px(CREATE_WIDTH), px(CREATE_HEIGHT)), |window, cx| {
        pin_theme(cx, false);
        cx.new(|cx| {
            let mut dialog = CreateNamespaceDialog::new(
                cluster,
                "kind-oxikube",
                creator,
                WeakEntity::new_invalid(),
                window,
                cx,
            );
            dialog.fill("Team C", "team=c", window, cx);
            dialog
        })
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

pub(crate) fn run() -> anyhow::Result<()> {
    check(
        "namespaces_table_dark",
        render_table(false)?,
        TABLE_WIDTH,
        TABLE_HEIGHT,
    )?;
    check(
        "namespaces_table_light",
        render_table(true)?,
        TABLE_WIDTH,
        TABLE_HEIGHT,
    )?;
    check(
        "namespaces_delete_dark",
        render_delete(false)?,
        DELETE_WIDTH,
        DELETE_HEIGHT,
    )?;
    check(
        "namespaces_delete_light",
        render_delete(true)?,
        DELETE_WIDTH,
        DELETE_HEIGHT,
    )?;
    check(
        "namespaces_create_dark",
        render_create()?,
        CREATE_WIDTH,
        CREATE_HEIGHT,
    )
}
