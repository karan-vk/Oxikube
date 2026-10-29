//! Namespaces on the real init path (E12-S16): the Namespaces section of the sidebar opens the
//! table with its pod counts, `u` uses a namespace (the session re-scopes its feeds), `p` pins
//! one and `0`-`9` jump to All or the nth favourite, all through the shipped keymap, the real
//! `CommandBus`, the `NamespaceService` and `StatePort`. Create and delete are guarded commands
//! on the same bus.

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::Resource;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_domain::session::NamespaceSelection;
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::{ResourceCall, TestPorts};
use serde_json::json;

use super::App;
use crate::app_state::AppState;

fn kind(name: &str, plural: &str, namespaced: bool) -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new("", "v1", name),
        preferred: true,
        plural: plural.into(),
        singular: name.to_lowercase(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch", "create", "delete"]),
        namespaced,
    }
}

fn namespace(name: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "Namespace",
        "metadata": { "name": name, "resourceVersion": "1" },
        "status": { "phase": "Active" },
    }))
    .unwrap()
}

impl App {
    fn selection(&mut self) -> NamespaceSelection {
        let state = self.vcx.update(|_, cx| AppState::global(cx));
        state
            .services()
            .sessions
            .get(&TestPorts::cluster_id())
            .expect("the session")
            .namespace_selection()
            .clone()
    }

    fn namespaces_table(&mut self) -> gpui::Entity<ResourceTable> {
        let ws = self.tab_workspace();
        self.vcx.update(|_, cx| {
            ws.read(cx)
                .items_of_type::<ResourceTable>()
                .into_iter()
                .find(|t| t.read(cx).gvk().kind.as_ref() == "Namespace")
                .expect("the Namespaces table is open")
        })
    }

    fn focus_namespaces(&mut self) {
        let table = self.namespaces_table();
        self.vcx.update(|window, cx| {
            let focus = gpui::Focusable::focus_handle(table.read(cx), cx);
            window.focus(&focus, cx);
        });
    }
}

/// An app on a cluster with namespaces `dev`, `prod` and a pod in each, the Namespaces table open.
fn open(cx: &mut TestAppContext) -> App {
    let mut app = App::start(cx, TestPorts::seeded());
    app.serve([
        kind("Pod", "pods", true),
        kind("Namespace", "namespaces", false),
    ]);
    let ports = app.ports.connector.ports_for(&TestPorts::cluster_id());
    for name in ["dev", "prod"] {
        ports.resources.insert(namespace(name));
    }
    let mut dev = oxikube_testkit::fixtures::pod_running();
    dev.meta.namespace = Some("dev".into());
    ports.resources.insert(dev);
    app.press("enter");
    app.tick();
    app.click("sidebar-entry-namespaces/namespaces");
    app.tick();
    app
}

#[gpui::test]
fn the_sidebar_opens_the_namespaces_table_with_pod_counts(cx: &mut TestAppContext) {
    let mut app = open(cx);
    let table = app.namespaces_table();
    let (names, pods) = app.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let pods = (0..d.rows().len())
                .map(|ix| {
                    d.provider()
                        .cell(
                            d.row(ix).unwrap(),
                            &oxikube_app::ColumnId::new("pods"),
                            jiff::Timestamp::now(),
                        )
                        .display()
                        .to_owned()
                })
                .collect::<Vec<_>>();
            (
                d.rows()
                    .iter()
                    .map(|r| r.name().to_owned())
                    .collect::<Vec<_>>(),
                pods,
            )
        })
    });
    assert_eq!(names, ["dev", "prod"]);
    assert_eq!(pods, ["1", "0"], "dev has the one pod");
    assert!(
        app.drawn("resource-table-create"),
        "the New button is there"
    );
}

#[gpui::test]
fn u_uses_the_namespace_and_the_feeds_follow(cx: &mut TestAppContext) {
    let mut app = open(cx);
    assert_eq!(app.selection(), NamespaceSelection::All);
    app.click("td-1-0"); // prod
    app.focus_namespaces();
    app.press("u");
    app.tick();
    assert_eq!(app.selection(), NamespaceSelection::single("prod"));

    // The session's feeds re-scoped: pods are now watched in `prod`.
    let ports = app.ports.connector.ports_for(&TestPorts::cluster_id());
    assert!(
        ports.resources.recorded_calls().iter().any(|call| matches!(
            call,
            ResourceCall::Watch { kind, namespace: Some(ns), .. }
                if &*kind.kind == "Pod" && ns == "prod"
        )),
        "a pods feed for prod was started: {:?}",
        ports.resources.recorded_calls()
    );
}

#[gpui::test]
fn favourites_persist_and_the_digits_jump_to_them(cx: &mut TestAppContext) {
    let mut app = open(cx);
    app.click("td-1-0"); // prod
    app.focus_namespaces();
    app.press("p"); // pins prod: favourite 1
    app.tick();
    app.click("td-0-0"); // dev
    app.focus_namespaces();
    app.press("p"); // favourite 2
    app.tick();

    app.focus_namespaces();
    app.press("1");
    app.tick();
    assert_eq!(app.selection(), NamespaceSelection::single("prod"));
    app.press("2");
    app.tick();
    assert_eq!(app.selection(), NamespaceSelection::single("dev"));
    app.press("0");
    app.tick();
    assert_eq!(
        app.selection(),
        NamespaceSelection::All,
        "0 is all namespaces"
    );
    app.press("9");
    app.tick();
    assert_eq!(
        app.selection(),
        NamespaceSelection::All,
        "a digit without a favourite changes nothing"
    );

    // Remembered per cluster in the state store, in order.
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let prefs = block_on(state.services().namespaces.prefs(&TestPorts::cluster_id())).unwrap();
    assert_eq!(prefs.favourites.iter().collect::<Vec<_>>(), ["prod", "dev"]);
}

#[gpui::test]
fn digits_are_text_while_the_filter_has_the_focus(cx: &mut TestAppContext) {
    let mut app = open(cx);
    app.focus_namespaces();
    app.press("/");
    app.press("1 2");
    app.tick();
    assert_eq!(
        app.selection(),
        NamespaceSelection::All,
        "typed into the filter, not a favourite"
    );
}

#[gpui::test]
fn create_and_delete_are_guarded_commands_on_the_bus(cx: &mut TestAppContext) {
    let mut app = open(cx);
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let ports = app.ports.connector.ports_for(&TestPorts::cluster_id());
    let cluster = TestPorts::cluster_id();
    for id in [CommandId::NAMESPACE_CREATE, CommandId::NAMESPACE_DELETE] {
        assert!(bus.tool(id).is_some(), "{id} has an MCP tool stub");
    }

    // An agent's create waits for a confirmation, and nothing reached the cluster.
    let create = Command::NamespaceCreate {
        cluster: cluster.clone(),
        name: "team-c".into(),
        labels: Default::default(),
    };
    let outcome = block_on(bus.dispatch(
        create,
        DispatchContext::new(Initiator::Agent, "agent").with_cluster(cluster.clone()),
    ))
    .expect("dispatches");
    assert!(
        matches!(outcome, oxikube_app::Outcome::NeedsConfirmation(_)),
        "{outcome:?}"
    );
    // Only the guard's dry run before it asks (E19-S07).
    assert!(
        ports
            .resources
            .mutating_calls()
            .iter()
            .all(ResourceCall::is_dry_run)
    );

    // Delete asks for the name.
    let delete = Command::NamespaceDelete {
        target: ResourceRef::cluster_scoped(
            cluster.clone(),
            Gvk::new("", "v1", "Namespace"),
            "prod",
        ),
        contents: Default::default(),
    };
    let outcome = block_on(bus.dispatch(
        delete,
        DispatchContext::new(Initiator::Agent, "agent").with_cluster(cluster),
    ))
    .expect("dispatches");
    let oxikube_app::Outcome::NeedsConfirmation(request) = outcome else {
        panic!("a delete is confirmed first");
    };
    assert_eq!(request.expected_name.as_deref(), Some("prod"));
    // Irreversible: the guard dry-ran it before asking, and that (with the create's dry run
    // above, E19-S07) is all that went out.
    assert!(request.preview.is_some(), "{request:?}");
    let calls = ports.resources.mutating_calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(calls.iter().all(ResourceCall::is_dry_run), "{calls:?}");
}

#[gpui::test]
fn ctrl_d_on_a_namespace_opens_the_delete_dialog(cx: &mut TestAppContext) {
    let mut app = open(cx);
    app.click("td-1-0");
    app.focus_namespaces();
    app.press("ctrl-d");
    app.tick();
    assert!(app.drawn("delete-dialog"));
    assert!(app.drawn("delete-type-input"), "the name is typed");
    let ports = app.ports.connector.ports_for(&TestPorts::cluster_id());
    assert!(ports.resources.mutating_calls().is_empty());
}

#[gpui::test]
fn new_on_the_namespaces_table_opens_the_create_dialog(cx: &mut TestAppContext) {
    let mut app = open(cx);
    app.click("resource-table-create");
    app.tick();
    assert!(app.drawn("namespace-create-dialog"));
}
