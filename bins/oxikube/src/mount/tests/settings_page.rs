//! The settings page in the running app (E21-S04): `settings::Open` (the palette's "Open
//! Settings", an MCP tool stub, `view::Open { view: "settings" }`) opens it as a tab of the main
//! window over the real init path, once; `cmd-,` / `ctrl-,`, the menu's action and the former
//! `oxikube::OpenPreferences` dispatch the same command.

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_settings_ui::page::SettingsPage;
use oxikube_testkit::TestPorts;
use oxikube_workspace::window::{OpenPreferences, settings};

use super::App;
use crate::app_state::AppState;

fn run(app: &mut App, command: Command) -> bool {
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let done = block_on(bus.dispatch(command, DispatchContext::new(Initiator::Ui, "me")));
    app.vcx.run_until_parked();
    done.is_ok()
}

fn pages(app: &mut App) -> usize {
    app.read(|ws, _| ws.items_of_type::<SettingsPage>()).len()
}

#[gpui::test]
fn the_command_opens_the_settings_tab_once(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    assert_eq!(pages(&mut app), 0);

    assert!(run(&mut app, Command::SettingsOpen));
    let opened = app.read(|ws, _| ws.items_of_type::<SettingsPage>());
    assert_eq!(opened.len(), 1);
    let active = app.read(|ws, cx| ws.active_item(cx).map(|item| item.item_id()));
    assert_eq!(active, Some(opened[0].entity_id()), "the new tab is shown");
    assert!(app.drawn("settings-nav"), "the page is drawn");
    assert!(
        app.drawn("settings-nav-general"),
        "its sections come from the schema"
    );

    // Asking again, by either command, shows the open tab instead of adding another.
    assert!(run(&mut app, Command::SettingsOpen));
    assert!(run(
        &mut app,
        Command::ViewOpen {
            view: "settings".into()
        }
    ));
    assert_eq!(pages(&mut app), 1);
}

#[gpui::test]
fn the_page_lists_the_registered_settings(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    assert!(run(&mut app, Command::SettingsOpen));
    let page = app.read(|ws, _| ws.items_of_type::<SettingsPage>())[0].clone();
    let (sections, terminal) = app.vcx.update(|_, cx| {
        let model = page.read(cx).model();
        (
            model.sections().len(),
            model.find_field("terminal.font_size").is_some(),
        )
    });
    assert!(sections > 5, "{sections} sections");
    assert!(terminal, "the terminal's settings have their section");
}

#[gpui::test]
fn the_key_and_the_menu_actions_dispatch_the_command(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.vcx.update(|window, _| window.activate_window());
    let key = if cfg!(target_os = "macos") {
        "cmd-,"
    } else {
        "ctrl-,"
    };
    app.press(key);
    assert_eq!(pages(&mut app), 1, "{key} opens the page");

    // The menu's action and the former preferences action reuse the open tab.
    app.vcx.dispatch_action(settings::Open);
    app.vcx.run_until_parked();
    app.vcx.dispatch_action(OpenPreferences);
    app.vcx.run_until_parked();
    assert_eq!(pages(&mut app), 1);
}

#[gpui::test]
fn the_command_is_in_the_bus_with_a_tool_stub(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let info = bus
        .all()
        .iter()
        .find(|info| info.id() == CommandId::SETTINGS_OPEN)
        .expect("settings::Open is registered");
    assert_eq!(info.meta().title, "Open Settings");
    assert!(
        bus.tool(CommandId::SETTINGS_OPEN).is_some(),
        "the MCP tool stub"
    );
    assert_eq!(CommandId::SETTINGS_OPEN.tool_name(), "app.settings_open");
}
