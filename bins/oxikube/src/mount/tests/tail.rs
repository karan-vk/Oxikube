//! "Tail in terminal (kubectl)" (E08-S08) in the running app: the log view's toolbar offers it
//! when kubectl is installed, and it opens a terminal tab in the cluster tab's bottom dock that runs
//! `kubectl logs -f` for the pod with the cluster's environment. The launcher and the kubectl
//! lookup are fakes (no process in a GPUI test); everything between them is the app's own mount.

use std::path::PathBuf;
use std::rc::Rc;

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_app::logs::kubectl::Kubectl;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::log::LogLine;
use oxikube_logs_ui::LogView;
use oxikube_resources_ui::table::ResourceTable;
use oxikube_terminal::view::{BackendDescriptor, TerminalServices, TerminalView};
use oxikube_testkit::{TestPorts, Timeline};
use oxikube_workspace::DockPosition;

use super::App;
use super::terminal::FakeLauncher;
use crate::app_state::AppState;
use crate::mount::logs::KubectlOverride;

const KUBECTL: &str = "/opt/tools/bin/kubectl";

/// The app with a fake terminal launcher and a lookup that finds kubectl at [`KUBECTL`] (or not).
fn start(cx: &mut TestAppContext, installed: bool) -> (App, Rc<FakeLauncher>) {
    let launcher = Rc::new(FakeLauncher::default());
    let for_terminals = launcher.clone();
    let app = App::start_with(cx, TestPorts::seeded(), move |cx| {
        oxikube_terminal::view::install(TerminalServices::new(for_terminals), cx);
        let lookup = move || installed.then(|| PathBuf::from(KUBECTL));
        cx.set_global(KubectlOverride(Kubectl::new(lookup)));
    });
    (app, launcher)
}

/// Opens the log of the running pod from its row, as "View Logs" does, and settles.
fn open_pod_log(app: &mut App) -> gpui::Entity<oxikube_workspace::Workspace> {
    let ports = app.ports.connector.ports_for(&TestPorts::cluster_id());
    let line = |i: i64| {
        LogLine::new(
            jiff::Timestamp::from_second(1_791_115_200 + i).unwrap(),
            "web-running",
            "app",
            format!("hello {i}"),
        )
    };
    ports
        .logs
        .script()
        .stream_logs
        .push_ok(Timeline::immediate((0..3).map(line)).keep_open());
    app.open_pods_table();
    let ws = app.tab_workspace();
    let table = app
        .vcx
        .update(|_, cx| ws.read(cx).items_of_type::<ResourceTable>().remove(0));
    let targets = app.vcx.update(|_, cx| table.read(cx).action_targets(cx));
    app.vcx.update(|window, cx| {
        table.update(cx, |table, cx| {
            table.run_action(CommandId::POD_VIEW_LOGS, targets, window, cx);
        });
    });
    app.tick();
    app.tick();
    ws
}

#[gpui::test]
fn the_logs_toolbar_tails_in_a_terminal_tab_of_the_cluster(cx: &mut TestAppContext) {
    let (mut app, launcher) = start(cx, true);
    let ws = open_pod_log(&mut app);
    let views = app
        .vcx
        .update(|_, cx| ws.read(cx).items_of_type::<LogView>());
    assert_eq!(views.len(), 1, "the log is open");
    app.click("log-overflow");
    assert!(
        app.drawn("log-tail-in-terminal"),
        "kubectl is installed: the toolbar's menu offers the action"
    );
    app.click("log-tail-in-terminal");
    app.tick();

    let cluster = TestPorts::cluster_id();
    let launches = launcher.launches.borrow().clone();
    let [
        BackendDescriptor::Local {
            cluster: tab_cluster,
            namespace,
            shell,
            args,
            title,
            ..
        },
    ] = &launches[..]
    else {
        panic!("one kubectl terminal started: {launches:?}");
    };
    assert_eq!(
        tab_cluster.as_ref(),
        Some(&cluster),
        "the cluster's environment"
    );
    assert_eq!(namespace.as_deref(), Some("demo"));
    assert_eq!(shell.as_deref(), Some(KUBECTL));
    assert_eq!(&args[..2], ["logs", "-f"]);
    assert!(args.iter().any(|a| a.starts_with("--context=")), "{args:?}");
    assert!(args.iter().any(|a| a == "--namespace=demo"), "{args:?}");
    assert_eq!(args.last().map(String::as_str), Some("web-running"));
    assert!(
        title
            .as_deref()
            .is_some_and(|t| t.starts_with("logs web-running"))
    );

    // A terminal tab in the cluster tab's bottom dock, like `terminal::New`'s.
    let (terminals, docked) = app.vcx.update(|_, cx| {
        let ws = ws.read(cx);
        let terminals = ws.items_of_type::<TerminalView>();
        let docked = terminals
            .first()
            .and_then(|view| ws.item_dock(view.entity_id(), cx));
        (terminals.len(), docked)
    });
    assert_eq!(terminals, 1);
    assert_eq!(docked, Some(DockPosition::Bottom));
}

#[gpui::test]
fn the_action_is_hidden_when_kubectl_is_missing(cx: &mut TestAppContext) {
    let (mut app, launcher) = start(cx, false);
    let ws = open_pod_log(&mut app);
    assert_eq!(
        app.vcx
            .update(|_, cx| ws.read(cx).items_of_type::<LogView>().len()),
        1
    );
    app.click("log-overflow");
    assert!(app.drawn("log-copy"), "the rest of the menu is there");
    assert!(!app.drawn("log-tail-in-terminal"), "hidden, not disabled");

    // The command still answers (a palette or an agent may send it): it says why, opens nothing.
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let target = ResourceRef::namespaced(
        TestPorts::cluster_id(),
        Gvk::new("", "v1", "Pod"),
        "demo",
        "web-running",
    );
    let outcome = block_on(bus.dispatch(
        Command::LogsTailInTerminal { target },
        DispatchContext::new(Initiator::Ui, "me"),
    ));
    assert!(
        outcome.is_ok(),
        "queued for the window like every log command"
    );
    app.tick();
    assert!(launcher.launches.borrow().is_empty(), "no terminal");
}

#[gpui::test]
fn the_command_is_on_the_bus_with_a_tool_stub_and_reads_only(cx: &mut TestAppContext) {
    let (app, _) = start(cx, true);
    let mut app = app;
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    let id = CommandId::LOGS_TAIL_IN_TERMINAL;
    assert_eq!(bus.owner(id), Some("oxikube_logs_ui"));
    let tool = bus.tool(id).expect("an MCP tool stub");
    assert_eq!(tool.name.as_str(), "k8s.logs_tail_in_terminal");
    let meta = bus.commands().find(|meta| meta.id == id).expect("declared");
    assert!(!meta.mutating, "kubectl logs only reads");
}
