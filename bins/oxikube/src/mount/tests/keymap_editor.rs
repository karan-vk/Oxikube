//! The keymap editor in the running app (E21-S07): `keymap::ShowEditor` on the bus with its tool
//! stub, the sidebar's Settings > Keymap entry opening the page in the connected cluster's tab,
//! and the page's "Open keymap.json" running `keymap::OpenUser` on the bus.

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_settings_ui::keymap::KeymapEditorView;
use oxikube_testkit::TestPorts;

use super::App;
use crate::app_state::AppState;

fn run(app: &mut App, command: Command) -> bool {
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let outcome = block_on(bus.dispatch(command, DispatchContext::new(Initiator::Ui, "me")));
    app.vcx.run_until_parked();
    outcome.is_ok()
}

fn pages(app: &mut App) -> usize {
    let tab = app.cluster_tabs().pop().expect("the cluster tab");
    app.vcx.update(|_, cx| {
        tab.read(cx)
            .workspace()
            .read(cx)
            .items_of_type::<KeymapEditorView>()
            .len()
    })
}

#[gpui::test]
fn the_command_is_on_the_bus_with_a_tool_stub(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    assert_eq!(
        bus.owner(CommandId::KEYMAP_SHOW_EDITOR),
        Some("oxikube_settings_ui::keymap")
    );
    assert_eq!(
        bus.tool(CommandId::KEYMAP_SHOW_EDITOR)
            .map(|t| t.name.as_str()),
        Some("app.keymap_show_editor")
    );
}

#[gpui::test]
fn the_sidebar_entry_and_the_command_open_one_page(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.press("enter");
    assert!(
        app.drawn("sidebar-section-settings"),
        "the Settings section is in the sidebar"
    );
    assert_eq!(pages(&mut app), 0, "nothing opens by itself");

    app.click("sidebar-entry-settings/keymap");
    assert_eq!(pages(&mut app), 1, "one page in the cluster's tab");
    assert!(app.drawn("keymap-editor"));
    assert!(app.drawn("keymap-table"));

    // Asking again (the palette's command) shows the open page.
    assert!(run(&mut app, Command::KeymapShowEditor));
    assert_eq!(pages(&mut app), 1);
}
