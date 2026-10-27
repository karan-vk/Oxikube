//! The Cloud section in the running app (E18-S06): `view::Open` with `cloud-clusters` opens it as
//! a tab over the real init path, once, and the four `cloud::*` commands are on the bus.

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_catalog_ui::CloudView;
use oxikube_catalog_ui::cloud::CLOUD_VIEW;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_testkit::TestPorts;

use super::App;
use crate::app_state::AppState;

fn open(app: &mut App) -> bool {
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let done = block_on(bus.dispatch(
        Command::ViewOpen {
            view: CLOUD_VIEW.to_owned(),
        },
        DispatchContext::new(Initiator::Ui, "me"),
    ));
    app.vcx.run_until_parked();
    done.is_ok()
}

#[gpui::test]
fn view_open_shows_the_cloud_section_once(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    assert!(app.read(|ws, _| ws.items_of_type::<CloudView>()).is_empty());

    assert!(open(&mut app));
    let views = app.read(|ws, _| ws.items_of_type::<CloudView>());
    assert_eq!(views.len(), 1);
    let active = app.read(|ws, cx| ws.active_item(cx).map(|item| item.item_id()));
    assert_eq!(active, Some(views[0].entity_id()), "the new tab is shown");

    assert!(open(&mut app));
    assert_eq!(app.read(|ws, _| ws.items_of_type::<CloudView>()).len(), 1);
}

#[gpui::test]
fn the_cloud_commands_are_on_the_bus_with_tool_stubs(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for id in [
        CommandId::CLOUD_REFRESH,
        CommandId::CLOUD_CANCEL,
        CommandId::CLOUD_ADD,
        CommandId::CLOUD_REMOVE,
    ] {
        assert_eq!(bus.owner(id), Some("oxikube_app::cloud"), "{id}");
        assert!(bus.tool(id).is_some(), "{id} has an MCP tool stub");
    }
}
