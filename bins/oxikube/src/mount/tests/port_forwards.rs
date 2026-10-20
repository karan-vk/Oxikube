//! The Port Forwards panel in the running app (E15-S05), over `TestPorts`: every command is on the
//! bus with its tool stub, and `:pf`, `f` in a table, the palette's command and the key all show
//! the panel in the window's bottom dock.

use gpui::TestAppContext;
use oxikube_domain::command::{Command, CommandId, ViewContext};
use oxikube_portforward_ui::PortForwardPanel;
use oxikube_testkit::TestPorts;
use oxikube_workspace::DockPosition;

use super::App;
use crate::app_state::AppState;

fn panel_open(app: &mut App) -> bool {
    app.read(|ws, cx| {
        ws.panel::<PortForwardPanel>().is_some()
            && ws
                .dock(DockPosition::Bottom, cx)
                .is_some_and(|dock| dock.is_open())
    })
}

fn toggle_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-shift-f"
    } else {
        "ctrl-shift-f"
    }
}

#[gpui::test]
fn the_commands_are_on_the_bus_with_tool_stubs(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for (id, owner) in [
        (CommandId::PORTFORWARD_START, "oxikube_app::portforward"),
        (CommandId::PORTFORWARD_STOP, "oxikube_app::portforward"),
        (CommandId::PORTFORWARD_RESTART, "oxikube_app::portforward"),
        (CommandId::PORTFORWARD_REMOVE, "oxikube_app::portforward"),
        (CommandId::PORTFORWARD_COPY_URL, "oxikube_portforward_ui"),
        (
            CommandId::PORTFORWARD_OPEN_IN_BROWSER,
            "oxikube_portforward_ui",
        ),
        (
            CommandId::PORTFORWARD_TOGGLE_PANEL,
            "oxikube_portforward_ui",
        ),
    ] {
        assert_eq!(bus.owner(id), Some(owner), "{id}");
        assert!(bus.tool(id).is_some(), "{id} has an MCP tool stub");
    }
}

#[gpui::test]
fn the_panel_starts_hidden_and_the_command_the_key_and_escape_toggle_it(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    assert!(!panel_open(&mut app), "hidden until asked for");
    super::notifications::dispatch(&mut app, Command::PortForwardTogglePanel { show: false });
    assert!(panel_open(&mut app), "the palette's command shows it");
    assert!(app.drawn("port-forwards-panel"));
    assert!(
        app.drawn("port-forwards-empty"),
        "no forwards yet: the hint"
    );
    app.press("escape");
    assert!(!panel_open(&mut app), "escape closes it");
    app.press(toggle_key());
    assert!(panel_open(&mut app), "the key shows it");
}

#[gpui::test]
fn the_palette_lists_the_toggle_in_a_table_and_f_and_pf_show_the_panel(cx: &mut TestAppContext) {
    let mut app = App::with_pods_table(cx);
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let session = state
        .services()
        .sessions
        .get(&TestPorts::cluster_id())
        .expect("connected");
    let ctx = oxikube_app::command_bus::CommandContext::in_session(
        ViewContext::Table,
        &oxikube_app::ActionContext::of(&session),
    );
    let listed: Vec<CommandId> = bus.list(&ctx).iter().map(|info| info.id()).collect();
    assert!(
        listed.contains(&CommandId::PORTFORWARD_TOGGLE_PANEL),
        "the palette offers it"
    );

    assert!(!panel_open(&mut app));
    app.press("f");
    assert!(panel_open(&mut app), "`f` in a table shows the panel");
    app.press("escape");
    assert!(!panel_open(&mut app));

    app.press("g");
    app.jump("pf");
    assert!(panel_open(&mut app), "`:pf` shows the panel");
    assert!(!app.bar_is_open());
}
