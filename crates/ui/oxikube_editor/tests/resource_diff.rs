//! The cross-cluster resource diff (E20-S09) in a workspace, end to end over the real
//! `DiffService`, `CommandBus` and `EditorViews` with the testkit's fake clusters on the
//! deterministic runtime:
//!
//! * `resource::DiffWith` with both objects opens a tab with the changed-path summary and the line
//!   diff, connecting the second cluster on demand;
//! * without a second object the "Diff with..." picker asks for it, and choosing sends the
//!   comparison;
//! * the status and name-and-namespace switches compare again; a missing side reads as all added;
//! * a read-only cluster can be compared and nothing is written to either cluster;
//! * an agent gets the changed-path summary and no tab, and a Secret's values are nowhere.

use std::rc::Rc;
use std::sync::Arc;

use futures::FutureExt as _;
use gpui::{App, Entity, TestAppContext, VisualTestContext, WeakEntity, Window};
use oxikube_app::ClusterSessionManager;
use oxikube_app::MutationGuard;
use oxikube_app::command_bus::{CommandBus, CommandRegistry, DispatchContext};
use oxikube_app::diff::{DiffService, IgnoreList};
use oxikube_app::session::SessionOptions;
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::safety::Initiator;
use oxikube_editor::view::{
    DiffState, DiffTargetDialog, EditorHost, EditorViewSink, EditorViews, EditorViewsDeps,
    ResourceDiffView, SchemaSource, register_diff_command,
};
use oxikube_ports::{ClusterContext, ResourceReader, SchemaPort, SourceId};
use oxikube_testkit::fakes::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeResourcePort,
    FakeSchemaPort, FakeStatePort,
};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, Workspace};
use serde_json::{Value, json};

fn context(name: &str) -> ClusterContext {
    let name = ContextName::new(name);
    ClusterContext::new(
        ClusterId::new("/kubeconfig", &name),
        name,
        SourceId("kubeconfig".into()),
    )
}

fn cluster(name: &str) -> ClusterId {
    context(name).cluster
}

fn config_map_gvk() -> Gvk {
    Gvk::new("", "v1", "ConfigMap")
}

fn config_ref(cluster_name: &str, namespace: &str, name: &str) -> ResourceRef {
    ResourceRef::namespaced(cluster(cluster_name), config_map_gvk(), namespace, name)
}

fn config_map(namespace: &str, name: &str, data: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": name, "namespace": namespace, "uid": "u", "resourceVersion": "7"},
        "data": data
    }))
    .expect("a ConfigMap")
}

fn secret(namespace: &str, name: &str, data: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": name, "namespace": namespace}, "data": data
    }))
    .expect("a Secret")
}

struct Schemas(Arc<FakeSchemaPort>);

impl SchemaSource for Schemas {
    fn cluster(&self) -> &ClusterId {
        static CLUSTER: std::sync::OnceLock<ClusterId> = std::sync::OnceLock::new();
        CLUSTER.get_or_init(|| cluster("a"))
    }
    fn label(&self, _: &App) -> gpui::SharedString {
        "a".into()
    }
    fn port(&self, _: &App) -> Option<Arc<dyn SchemaPort>> {
        Some(self.0.clone())
    }
}

struct Host {
    workspace: WeakEntity<Workspace>,
}

impl EditorHost for Host {
    fn active_cluster(&self, _: &App) -> Option<ClusterId> {
        Some(cluster("a"))
    }
    fn workspace(&self, _: &ClusterId, _: &App) -> Option<Entity<Workspace>> {
        self.workspace.upgrade()
    }
    fn show(&self, _: &ClusterId, _: &mut Window, _: &mut App) {}
    fn schemas(&self, _: &ClusterId) -> Rc<dyn SchemaSource> {
        Rc::new(Schemas(Arc::new(FakeSchemaPort::new())))
    }
    fn reader(&self, _: &ClusterId) -> Option<Arc<dyn ResourceReader>> {
        None
    }
}

/// Sends every command to the real bus as `initiator`; the fakes answer at once.
struct BusDispatcher {
    bus: CommandBus,
    initiator: Initiator,
}

impl CommandDispatcher for BusDispatcher {
    fn dispatch(&self, command: Command, _: &mut App) {
        let context = DispatchContext::new(self.initiator, "me");
        // A failed command is the tab's `Failed` state; the real dispatcher shows a toast.
        self.bus
            .dispatch(command, context)
            .now_or_never()
            .expect("the fakes answer at once")
            .ok();
    }
}

struct Fixture {
    workspace: Entity<Workspace>,
    vcx: VisualTestContext,
    sessions: ClusterSessionManager,
    a: Arc<FakeResourcePort>,
    b: Arc<FakeResourcePort>,
    bus: CommandBus,
    dispatcher: Rc<dyn CommandDispatcher>,
    ignore: IgnoreList,
    _views: Entity<EditorViews>,
}

/// Clusters `a` (connected, `read_only` as asked) and `b` (in the catalog, never opened).
fn setup(cx: &mut TestAppContext, read_only: bool) -> Fixture {
    let (workspace, mut vcx) = open_workspace(cx);
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([context("a"), context("b")]));
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(connector.clone(), source, clock.clone());
    sessions.open(
        &context("a"),
        SessionOptions {
            read_only,
            ..SessionOptions::default()
        },
    );
    sessions
        .connect(&cluster("a"))
        .now_or_never()
        .expect("instant")
        .expect("connected");
    let a = connector.ports_for(&cluster("a")).resources;
    let ignore = IgnoreList::default();
    let (sink, requests) = EditorViewSink::channel();
    let mut registry = CommandRegistry::new();
    let service = DiffService::new(sessions.clone(), ignore.clone());
    registry
        .install("oxikube_editor::diff", |r| {
            register_diff_command(r, sink.clone(), service)
        })
        .expect("registered");
    let guard = MutationGuard::new(sessions.clone(), Arc::new(FakeStatePort::new()), clock);
    let bus = CommandBus::new(registry, guard);
    let dispatcher: Rc<dyn CommandDispatcher> = Rc::new(BusDispatcher {
        bus: bus.clone(),
        initiator: Initiator::Ui,
    });
    let views = vcx.update(|window, cx| {
        oxikube_runtime::init_deterministic(cx);
        oxikube_editor::init(cx);
        let deps = EditorViewsDeps {
            host: Rc::new(Host {
                workspace: workspace.downgrade(),
            }),
            window_workspace: workspace.downgrade(),
            dispatcher: dispatcher.clone(),
        };
        EditorViews::start(deps, requests, window, cx)
    });
    vcx.run_until_parked();
    // `b` is connected by the diff itself; its port exists once it is.
    let b = connector.ports_for(&cluster("b")).resources;
    Fixture {
        workspace,
        vcx,
        sessions,
        a,
        b,
        bus,
        dispatcher,
        ignore,
        _views: views,
    }
}

impl Fixture {
    fn diff(&mut self, target: ResourceRef, other: Option<ResourceRef>) {
        let dispatcher = self.dispatcher.clone();
        self.vcx.update(|_, cx| {
            dispatcher.dispatch(
                Command::ResourceDiffWith {
                    target,
                    other,
                    include_status: false,
                    compare_identity: false,
                },
                cx,
            );
        });
        self.vcx.run_until_parked();
    }

    fn view(&mut self) -> Option<Entity<ResourceDiffView>> {
        self.vcx.read(|cx| {
            self.workspace
                .read(cx)
                .items_of_type::<ResourceDiffView>()
                .pop()
        })
    }

    fn ready(&mut self) -> Arc<oxikube_editor::view::DiffData> {
        let view = self.view().expect("a diff tab");
        match self.vcx.read(|cx| view.read(cx).state().clone()) {
            DiffState::Ready(data) => data,
            other => panic!("not ready: {other:?}"),
        }
    }
}

#[gpui::test]
fn a_comparison_across_clusters_opens_a_tab_and_connects_the_other_cluster(
    cx: &mut TestAppContext,
) {
    let mut f = setup(cx, false);
    f.a.insert(config_map(
        "shop",
        "app",
        json!({"url": "http://stg", "mode": "x"}),
    ));
    f.b.insert(config_map(
        "shop",
        "app",
        json!({"url": "http://prod", "mode": "x"}),
    ));
    assert!(f.sessions.get(&cluster("b")).is_none(), "b is not open yet");

    f.diff(
        config_ref("a", "shop", "app"),
        Some(config_ref("b", "shop", "app")),
    );

    let data = f.ready();
    assert!(
        f.sessions
            .get(&cluster("b"))
            .expect("b opened")
            .is_connected(),
        "connected on demand"
    );
    assert_eq!(data.diff.changes.paths.len(), 1);
    assert_eq!(
        data.diff.changes.paths[0].line(),
        r#"data.url: "http://stg" -> "http://prod""#
    );
    let (removed, added) = data.lines.counts();
    assert_eq!((removed, added), (1, 1), "only the changed line differs");
    assert_eq!(data.lines.hunks(), 1);
}

#[gpui::test]
fn the_same_comparison_again_updates_the_tab_instead_of_opening_another(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.a.insert(config_map("x", "app", json!({"k": "1"})));
    f.a.insert(config_map("y", "app", json!({"k": "2"})));
    let (left, right) = (config_ref("a", "x", "app"), config_ref("a", "y", "app"));
    f.diff(left.clone(), Some(right.clone()));
    f.diff(left, Some(right));
    let count = f.vcx.read(|cx| {
        f.workspace
            .read(cx)
            .items_of_type::<ResourceDiffView>()
            .len()
    });
    assert_eq!(count, 1);
}

#[gpui::test]
fn without_a_second_object_the_picker_asks_and_choosing_compares(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.a.insert(config_map("x", "app", json!({"k": "1"})));
    f.a.insert(config_map("y", "app", json!({"k": "2"})));
    f.diff(config_ref("a", "x", "app"), None);
    assert!(f.view().is_none(), "no tab until a choice is made");
    let dialog = f
        .vcx
        .read(|cx| {
            f.workspace
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<DiffTargetDialog>()
        })
        .expect("the picker is open");
    assert_eq!(
        dialog.read_with(&f.vcx, |d, _| d.cluster().clone()),
        cluster("a")
    );
    // The namespace field starts at the object's; edit it, then compare.
    dialog.update_in(&mut f.vcx, |dialog, window, cx| {
        dialog.set_namespace("y", window, cx);
        dialog.submit(window, cx);
    });
    f.vcx.run_until_parked();
    let data = f.ready();
    assert_eq!(data.diff.right.target, config_ref("a", "y", "app"));
    assert_eq!(data.diff.changes.paths[0].line(), r#"data.k: "1" -> "2""#);
}

#[gpui::test]
fn the_picker_refuses_an_empty_name(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.diff(config_ref("a", "x", "app"), None);
    let dialog = f
        .vcx
        .read(|cx| {
            f.workspace
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<DiffTargetDialog>()
        })
        .expect("the picker is open");
    dialog.update_in(&mut f.vcx, |dialog, window, cx| {
        dialog.set_name("  ", window, cx);
        dialog.submit(window, cx);
    });
    f.vcx.run_until_parked();
    assert!(f.view().is_none());
    assert!(dialog.read_with(&f.vcx, |d, _| d.problem().is_some()));
}

#[gpui::test]
fn the_status_and_identity_switches_compare_again(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let with_status = |ns: &str, phase: &str| {
        Resource::from_json(json!({
            "apiVersion": "v1", "kind": "ConfigMap",
            "metadata": {"name": "app", "namespace": ns},
            "data": {"k": "v"}, "status": {"phase": phase}
        }))
        .expect("an object")
    };
    f.a.insert(with_status("x", "one"));
    f.a.insert(with_status("y", "two"));
    f.diff(
        config_ref("a", "x", "app"),
        Some(config_ref("a", "y", "app")),
    );
    assert!(
        f.ready().diff.is_identical(),
        "status and namespace are ignored by default"
    );

    let view = f.view().expect("tab");
    view.update(&mut f.vcx, |view, cx| view.toggle_status(cx));
    f.vcx.run_until_parked();
    let lines: Vec<_> = f
        .ready()
        .diff
        .changes
        .paths
        .iter()
        .map(|c| c.line())
        .collect();
    assert_eq!(lines, [r#"status.phase: "one" -> "two""#]);

    view.update(&mut f.vcx, |view, cx| view.toggle_identity(cx));
    f.vcx.run_until_parked();
    let lines: Vec<_> = f
        .ready()
        .diff
        .changes
        .paths
        .iter()
        .map(|c| c.line())
        .collect();
    assert_eq!(
        lines,
        [
            r#"metadata.namespace: "x" -> "y""#,
            r#"status.phase: "one" -> "two""#
        ]
    );
}

#[gpui::test]
fn a_missing_object_shows_the_other_as_all_added(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.a.insert(config_map("prod", "app", json!({"k": "v"})));
    f.diff(
        config_ref("a", "stg", "app"),
        Some(config_ref("a", "prod", "app")),
    );
    let data = f.ready();
    assert!(!data.diff.left.exists() && data.diff.right.exists());
    let (removed, added) = data.lines.counts();
    assert_eq!(removed, 0);
    assert!(added > 0);
    assert!(
        data.summary[0].contains("does not exist"),
        "{:?}",
        data.summary
    );
}

#[gpui::test]
fn a_failed_comparison_says_why_in_the_tab(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.diff(
        config_ref("a", "x", "app"),
        Some(config_ref("a", "y", "app")),
    );
    let view = f.view().expect("tab");
    let state = f.vcx.read(|cx| view.read(cx).state().clone());
    let DiffState::Failed(text) = state else {
        panic!("{state:?}");
    };
    assert!(text.contains("neither"), "{text}");
}

#[gpui::test]
fn a_read_only_cluster_can_be_compared_and_nothing_is_written(cx: &mut TestAppContext) {
    let mut f = setup(cx, true);
    f.a.insert(config_map("x", "app", json!({"k": "1"})));
    f.a.insert(config_map("y", "app", json!({"k": "2"})));
    f.diff(
        config_ref("a", "x", "app"),
        Some(config_ref("a", "y", "app")),
    );
    assert!(!f.ready().diff.is_identical());
    assert!(f.a.mutating_calls().is_empty() && f.b.mutating_calls().is_empty());
}

#[gpui::test]
fn the_ignore_setting_applies_to_the_next_comparison(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.a.insert(config_map("x", "app", json!({"k": "1"})));
    f.a.insert(config_map("y", "app", json!({"k": "2"})));
    f.diff(
        config_ref("a", "x", "app"),
        Some(config_ref("a", "y", "app")),
    );
    assert!(!f.ready().diff.is_identical());
    f.ignore
        .set(oxikube_app::diff::IgnorePaths::new(&["/data/k"]));
    let view = f.view().expect("tab");
    view.update(&mut f.vcx, |view, cx| view.refresh(cx));
    f.vcx.run_until_parked();
    assert!(f.ready().diff.is_identical());
}

#[gpui::test]
fn an_agent_gets_the_summary_without_a_tab_and_no_secret_value(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.a.insert(secret("x", "db", json!({"password": "c2VjcmV0LW9uZQ=="})));
    f.a.insert(secret("y", "db", json!({"password": "c2VjcmV0LXR3bw=="})));
    let secret_ref =
        |ns: &str| ResourceRef::namespaced(cluster("a"), Gvk::new("", "v1", "Secret"), ns, "db");
    let command = Command::ResourceDiffWith {
        target: secret_ref("x"),
        other: Some(secret_ref("y")),
        include_status: false,
        compare_identity: false,
    };
    let output = f
        .bus
        .dispatch(command, DispatchContext::new(Initiator::Agent, "agent"))
        .now_or_never()
        .expect("instant")
        .expect("ran")
        .completed()
        .expect("no confirmation for a read");
    f.vcx.run_until_parked();
    assert!(f.view().is_none(), "an agent's call opens no tab");
    let text = format!("{:?}{:?}", output.message, output.data);
    assert!(text.contains("data.password: changed"), "{text}");
    assert!(!text.contains("c2VjcmV0"), "{text}");
    // An agent must name the second object itself.
    let missing = Command::ResourceDiffWith {
        target: secret_ref("x"),
        other: None,
        include_status: false,
        compare_identity: false,
    };
    assert!(
        f.bus
            .dispatch(missing, DispatchContext::new(Initiator::Agent, "agent"))
            .now_or_never()
            .expect("instant")
            .is_err()
    );
}

#[gpui::test]
fn a_secret_diff_tab_holds_no_value(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.a.insert(secret(
        "x",
        "db",
        json!({"password": "c2VjcmV0LW9uZQ==", "same": "c2FtZQ=="}),
    ));
    f.a.insert(secret(
        "y",
        "db",
        json!({"password": "c2VjcmV0LXR3bw==", "same": "c2FtZQ=="}),
    ));
    let secret_ref =
        |ns: &str| ResourceRef::namespaced(cluster("a"), Gvk::new("", "v1", "Secret"), ns, "db");
    f.diff(secret_ref("x"), Some(secret_ref("y")));
    let data = f.ready();
    let all = format!("{:?}", data);
    for value in ["c2VjcmV0", "c2FtZQ"] {
        assert!(!all.contains(value), "{value} in the tab: {all}");
    }
    let (removed, added) = data.lines.counts();
    assert_eq!((removed, added), (1, 1), "only the changed key differs");
}

#[gpui::test]
fn the_tab_draws_the_summary_and_only_the_visible_rows(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    // 400 keys, one of which differs: the diff folds the unchanged stretches.
    let data = |last: &str| {
        let mut map: serde_json::Map<String, Value> = (0..400)
            .map(|i| (format!("key-{i:03}"), json!("v")))
            .collect();
        map.insert("key-399".into(), json!(last));
        Value::Object(map)
    };
    f.a.insert(config_map("x", "app", data("a")));
    f.a.insert(config_map("y", "app", data("b")));
    f.diff(
        config_ref("a", "x", "app"),
        Some(config_ref("a", "y", "app")),
    );
    let view = f.view().expect("tab");
    f.vcx.run_until_parked();
    for selector in [
        "resource-diff-title",
        "resource-diff-counts",
        "resource-diff-summary",
    ] {
        assert!(
            f.vcx.debug_bounds(selector).is_some(),
            "{selector} is drawn"
        );
    }
    assert!(f.vcx.debug_bounds("resource-diff-message").is_none());
    let data = f.ready();
    // The line diff holds the changed line and its context, not the 400 keys.
    let rows = data.lines.side_rows().len();
    assert!(rows < 20, "{rows} rows: unchanged stretches are folded");
    let _ = view;
}
