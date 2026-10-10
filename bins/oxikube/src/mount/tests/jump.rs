//! The `:` jump bar on the real init path (E11-S05): `:` in a resource table opens it, a line runs
//! as navigation commands on the app's bus (the list opens, the filter lands in the table), `[`
//! and `]` replay the history, and `:q` goes through the app's quit command.

use gpui::TestAppContext;
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_palette::jump::JumpBar;
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::TestPorts;
use oxikube_ui::dialog::OverlayExt as _;

use super::App;
use crate::app_state::AppState;

fn kind(group: &str, name: &str, plural: &str) -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new(group, "v1", name),
        preferred: true,
        plural: plural.into(),
        singular: name.to_lowercase(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch"]),
        namespaced: true,
    }
}

fn namespace(name: &str) -> oxikube_domain::Resource {
    oxikube_domain::Resource::from_json(serde_json::json!({
        "apiVersion": "v1",
        "kind": "Namespace",
        "metadata": { "name": name },
    }))
    .expect("namespace json")
}

impl App {
    /// A connected cluster with its Pods table open and focused (the view `:` is pressed in).
    fn with_pods_table(cx: &mut TestAppContext) -> Self {
        let mut app = App::start(cx, TestPorts::seeded());
        // The cluster's own namespace list, which a namespace in a line is checked against.
        let ports = app.ports.connector.ports_for(&TestPorts::cluster_id());
        for name in ["default", "kube-system"] {
            ports.resources.insert(namespace(name));
        }
        app.serve([
            kind("", "Pod", "pods"),
            kind("", "ConfigMap", "configmaps"),
            kind("apps", "Deployment", "deployments"),
        ]);
        app.press("enter");
        app.tick();
        app.click("sidebar-entry-workloads/pods");
        app.tick();
        assert_eq!(app.table_kinds(), ["Pod"]);
        app
    }

    fn table_kinds(&mut self) -> Vec<String> {
        self.tables_of(|table| table.gvk().kind.to_string())
    }

    fn tables_of<R>(&mut self, f: impl Fn(&ResourceTable) -> R) -> Vec<R> {
        let ws = self.tab_workspace();
        self.vcx.update(|_, cx| {
            ws.read(cx)
                .items_of_type::<ResourceTable>()
                .into_iter()
                .map(|table| f(table.read(cx)))
                .collect()
        })
    }

    /// The kind of the table the cluster tab shows, if it shows a table.
    fn shown_kind(&mut self) -> Option<String> {
        let ws = self.tab_workspace();
        self.vcx.update(|_, cx| {
            let active = ws.read(cx).active_item(cx)?.item_id();
            ws.read(cx)
                .items_of_type::<ResourceTable>()
                .into_iter()
                .find(|table| table.entity_id() == active)
                .map(|table| table.read(cx).gvk().kind.to_string())
        })
    }

    fn bar_is_open(&mut self) -> bool {
        let ws = self.workspace();
        self.vcx.update(|_, cx| {
            ws.read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<JumpBar>()
                .is_some()
        })
    }

    /// Opens the bar with `:`, types `line` and presses Enter.
    fn jump(&mut self, line: &str) {
        self.press(":");
        assert!(self.bar_is_open(), "`:` opened the jump bar");
        // The contexts and the namespaces are read in the background when the bar opens.
        self.tick();
        self.vcx.simulate_input(line);
        self.vcx.run_until_parked();
        self.press("enter");
        self.tick();
    }
}

#[gpui::test]
fn colon_in_a_table_opens_the_bar_and_a_line_opens_the_list(cx: &mut TestAppContext) {
    let mut app = App::with_pods_table(cx);
    app.jump("cm");
    assert!(!app.bar_is_open(), "a good line closes the bar");
    let mut kinds = app.table_kinds();
    kinds.sort();
    assert_eq!(
        kinds,
        ["ConfigMap", "Pod"],
        "`:cm` opened the ConfigMaps table"
    );
}

#[gpui::test]
fn an_alias_of_a_kind_with_a_namespace_and_a_filter_narrows_the_new_table(cx: &mut TestAppContext) {
    let mut app = App::with_pods_table(cx);
    app.jump("deploy default /api app=x");
    assert!(app.table_kinds().contains(&"Deployment".to_owned()));
    let filters = app.tables_of(|t| (t.gvk().kind.to_string(), t.filter_parts().clone()));
    let (_, parts) = filters
        .into_iter()
        .find(|(kind, _)| kind == "Deployment")
        .expect("the Deployments table");
    assert!(
        parts.filter.pattern.is_some(),
        "the /api name filter reached the table"
    );
    assert_eq!(
        parts.selector.as_ref().map(ToString::to_string).as_deref(),
        Some("app=x"),
        "and the selector, applied by the server"
    );
}

#[gpui::test]
fn a_mistake_keeps_the_bar_open_and_nothing_opens(cx: &mut TestAppContext) {
    let mut app = App::with_pods_table(cx);
    app.jump("podz");
    assert!(app.bar_is_open(), "an unknown alias stays in the bar");
    assert!(app.drawn("jump-problem"), "with the problem shown");
    assert_eq!(app.table_kinds(), ["Pod"]);
    app.press("escape");
    assert!(!app.bar_is_open());
}

#[gpui::test]
fn the_bracket_keys_replay_the_history(cx: &mut TestAppContext) {
    let mut app = App::with_pods_table(cx);
    app.jump("cm");
    app.jump("deploy");
    assert_eq!(app.shown_kind().as_deref(), Some("Deployment"));
    // `[` in a table: back to `cm`, whose table is open already, so it is shown again.
    app.press("[");
    app.tick();
    assert!(!app.bar_is_open());
    assert_eq!(app.shown_kind().as_deref(), Some("ConfigMap"));
    assert_eq!(app.toasts(), Vec::<String>::new());
    app.press("[");
    app.tick();
    // Nothing before `cm`: the table stays, `cm` is not run again, and the user is told.
    assert_eq!(app.shown_kind().as_deref(), Some("ConfigMap"));
    assert_eq!(app.toasts(), ["Nothing earlier in the jump history."]);
    assert_eq!(app.table_kinds().len(), 3, "no table was added");
    // `]` goes forward again, and `-` flips between the last two views.
    app.press("]");
    app.tick();
    assert_eq!(app.shown_kind().as_deref(), Some("Deployment"));
    app.press("-");
    app.tick();
    assert_eq!(app.shown_kind().as_deref(), Some("ConfigMap"));
    app.press("-");
    app.tick();
    assert_eq!(app.shown_kind().as_deref(), Some("Deployment"));
}

#[gpui::test]
fn the_jump_commands_and_quit_are_on_the_bus_with_tool_stubs(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let bus = app
        .vcx
        .update(|_, cx| AppState::global(cx).command_bus().cloned())
        .expect("the mount set the bus");
    for id in [
        CommandId::PALETTE_OPEN_JUMP,
        CommandId::JUMP_BACK,
        CommandId::JUMP_FORWARD,
        CommandId::JUMP_LAST,
        CommandId::APP_QUIT,
    ] {
        assert!(bus.is_registered(id), "{id}");
        assert!(bus.tool(id).is_some(), "{id} has an MCP tool stub");
    }
}

#[gpui::test]
fn q_reaches_the_quit_guard_through_the_apps_bus_and_window(cx: &mut TestAppContext) {
    let mut app = App::with_pods_table(cx);
    // With an operation running, a quit asks first: the dialog is how the test sees that `:q`
    // went bar -> `app::Quit` on the bus -> the window's quit queue (`serve_quit`) ->
    // `request_quit` (the platform's own quit does nothing observable in a test).
    app.vcx.update(|_, cx| {
        oxikube_workspace::session::register_operation_provider(cx, |_| {
            vec![oxikube_workspace::session::RunningOperation::new(
                "Exec session",
                "pod/web-0 in prod",
            )]
        });
    });
    assert!(!app.vcx.update(|window, cx| window.has_active_dialog(cx)));
    app.jump("q");
    app.tick();
    assert!(!app.bar_is_open());
    assert!(
        app.vcx.update(|window, cx| window.has_active_dialog(cx)),
        "the quit asked before ending the running exec session; toasts: {:?}",
        app.toasts()
    );
}
