//! The command palette on the real init path (E11-S03): the key opens it over the focused view,
//! it lists the commands the app's bus registered (not test handlers) for that view, and
//! confirming one runs it through the bus and the same handlers a key or a button reaches.

use gpui::TestAppContext;
use oxikube_app::command_bus::{CommandContext, Selection};
use oxikube_domain::command::{Command, CommandId, ViewContext};
use oxikube_domain::ids::Gvk;
use oxikube_logs_ui::LogView;
use oxikube_palette::CommandPalette;
use oxikube_testkit::TestPorts;

use super::App;
use crate::app_state::AppState;

/// The key that opens the palette on this OS (the shipped keymap's).
const OPEN_KEY: &str = if cfg!(target_os = "macos") {
    "cmd-shift-p"
} else {
    "ctrl-shift-p"
};

impl App {
    fn palette(&mut self) -> Option<gpui::Entity<CommandPalette>> {
        let workspace = self.workspace();
        self.vcx.update(|_, cx| {
            workspace
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<CommandPalette>()
        })
    }

    fn listed(&mut self) -> Vec<CommandId> {
        let palette = self.palette().expect("the palette is open");
        self.vcx
            .update(|_, cx| palette.read(cx).picker().read(cx).delegate.listed())
    }
}

#[gpui::test]
fn the_shortcut_opens_the_palette_on_the_catalog_with_what_runs_there(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.press(OPEN_KEY);
    assert!(app.palette().is_some(), "{OPEN_KEY} opens the palette");
    let listed = app.listed();
    // No cluster yet: the commands of the catalog and the global ones, not a namespace pick.
    assert!(listed.contains(&CommandId::CLUSTER_CONNECT));
    assert!(listed.contains(&CommandId::PALETTE_TOGGLE));
    assert!(!listed.contains(&CommandId::NAMESPACE_SELECT));
    assert!(!listed.contains(&CommandId::POD_VIEW_LOGS));
    app.press(OPEN_KEY);
    assert!(app.palette().is_none(), "the shortcut closes it again");
}

#[gpui::test]
fn the_palette_lists_exactly_what_the_bus_lists_for_the_view(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_pods_table();
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the mount set the bus").clone();

    app.press(OPEN_KEY);
    let listed = app.listed();
    // Same index, same availability: the palette shows `CommandBus::list` for a Table with a pod
    // selected on the connected (writable) cluster.
    let session = state
        .services()
        .sessions
        .get(&TestPorts::cluster_id())
        .expect("connected");
    let mut ctx = CommandContext::in_session(
        ViewContext::Table,
        &oxikube_app::ActionContext::of(&session),
    );
    ctx.selection = Selection::one(Gvk::new("", "v1", "Pod"));
    let mut expected: Vec<CommandId> = bus.list(&ctx).iter().map(|info| info.id()).collect();
    expected.sort();
    let mut listed_sorted = listed.clone();
    listed_sorted.sort();
    assert_eq!(listed_sorted, expected);
    assert!(listed.contains(&CommandId::POD_VIEW_LOGS));
    assert!(listed.contains(&CommandId::RESOURCE_DELETE));
}

#[gpui::test]
fn confirming_view_logs_in_the_palette_opens_the_pods_log(cx: &mut TestAppContext) {
    use oxikube_domain::log::LogLine;
    use oxikube_testkit::Timeline;

    let mut app = App::start(cx, TestPorts::seeded());
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
        .push_ok(Timeline::immediate((0..3).map(line)));
    app.open_pods_table();

    app.press(OPEN_KEY);
    app.vcx.simulate_input("pod view logs");
    app.vcx.run_until_parked();
    app.press("enter");
    app.tick();
    app.tick();

    assert!(app.palette().is_none(), "confirming closes the palette");
    let ws = app.tab_workspace();
    let views = app
        .vcx
        .update(|_, cx| ws.read(cx).items_of_type::<LogView>());
    assert_eq!(views.len(), 1, "the palette ran pod::ViewLogs on the row");
    // It is remembered for every window's palette; the log view that has the focus now lists the
    // log commands, not the pod table's.
    let recent = app
        .vcx
        .update(|_, cx| AppState::global(cx).recents().recent());
    assert_eq!(recent, [CommandId::POD_VIEW_LOGS]);
    app.press(OPEN_KEY);
    let listed = app.listed();
    assert!(
        listed.contains(&CommandId::LOGS_FIND),
        "the focused log view's commands"
    );
    assert!(
        !listed.contains(&CommandId::POD_VIEW_LOGS),
        "a pod table's are not"
    );
}

#[gpui::test]
fn a_read_only_cluster_hides_the_mutations_and_show_all_marks_them(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_pods_table();
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the mount set the bus").clone();
    let cluster = TestPorts::cluster_id();
    let workspace = app.workspace();
    let runner = oxikube_workspace::ClusterCommandRunner::new(
        bus,
        state.services().sessions.clone(),
        "me",
        &workspace,
    );
    app.vcx.update(|window, cx| {
        runner.run(
            Command::ClusterToggleReadOnly {
                cluster: cluster.clone(),
                read_only: Some(true),
            },
            window,
            cx,
        );
    });
    app.tick();

    app.press(OPEN_KEY);
    let listed = app.listed();
    assert!(
        !listed.contains(&CommandId::RESOURCE_DELETE),
        "hidden, not greyed"
    );
    assert!(listed.contains(&CommandId::POD_VIEW_LOGS));
    let shown = if cfg!(target_os = "macos") {
        "cmd-shift-a"
    } else {
        "ctrl-shift-a"
    };
    app.press(shown);
    let all = app.listed();
    assert!(
        all.contains(&CommandId::RESOURCE_DELETE),
        "show all lists it"
    );
}

#[gpui::test]
fn the_bus_commands_open_and_toggle_the_palette(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the mount set the bus").clone();
    for id in [
        CommandId::PALETTE_TOGGLE,
        CommandId::PALETTE_TOGGLE_SHOW_ALL,
    ] {
        assert!(bus.is_registered(id), "{id} is on the bus");
        assert!(bus.tool(id).is_some(), "{id} has its MCP tool stub");
    }
    let workspace = app.workspace();
    let runner = oxikube_workspace::ClusterCommandRunner::new(
        bus,
        state.services().sessions.clone(),
        "agent",
        &workspace,
    );
    app.vcx
        .update(|window, cx| runner.run(Command::PaletteToggle, window, cx));
    app.tick();
    assert!(
        app.palette().is_some(),
        "palette::Toggle opens it from the bus"
    );
    app.vcx
        .update(|window, cx| runner.run(Command::PaletteToggleShowAll, window, cx));
    app.tick();
    let show_all = {
        let palette = app.palette().expect("still open");
        app.vcx
            .update(|_, cx| palette.read(cx).picker().read(cx).delegate.show_all())
    };
    assert!(show_all, "palette::ToggleShowAll flips the open palette");
    app.vcx
        .update(|window, cx| runner.run(Command::PaletteToggle, window, cx));
    app.tick();
    assert!(app.palette().is_none(), "palette::Toggle closes it");
}
