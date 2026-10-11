//! The chart gallery in the running app (E13-S07): `charts::OpenGallery` ("Open Chart Gallery" in
//! the palette, an MCP tool stub) opens it as a tab over the real init path, once; the page draws
//! every chart state.

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_testkit::TestPorts;

use super::App;
use crate::app_state::AppState;
use crate::mount::gallery::ChartGalleryPage;

fn open(app: &mut App) -> bool {
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let done = block_on(bus.dispatch(
        Command::ChartsOpenGallery,
        DispatchContext::new(Initiator::Ui, "me"),
    ));
    app.vcx.run_until_parked();
    done.is_ok()
}

#[gpui::test]
fn the_command_opens_the_gallery_tab_once(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    assert!(
        app.read(|ws, _| ws.items_of_type::<ChartGalleryPage>())
            .is_empty()
    );

    assert!(open(&mut app));
    let pages = app.read(|ws, _| ws.items_of_type::<ChartGalleryPage>());
    assert_eq!(pages.len(), 1);
    let active = app.read(|ws, cx| ws.active_item(cx).map(|item| item.item_id()));
    assert_eq!(active, Some(pages[0].entity_id()), "the new tab is shown");

    // Asking again shows the open tab instead of adding another.
    assert!(open(&mut app));
    assert_eq!(
        app.read(|ws, _| ws.items_of_type::<ChartGalleryPage>())
            .len(),
        1
    );
}

#[gpui::test]
fn the_page_draws_its_charts(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    assert!(open(&mut app));
    assert!(
        app.drawn("chart-plot"),
        "the gallery's charts are laid out in the window"
    );
}

#[gpui::test]
fn the_command_is_in_the_bus_with_a_tool_stub(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    assert!(
        bus.all()
            .iter()
            .any(|info| info.id() == CommandId::CHARTS_OPEN_GALLERY)
    );
    assert!(
        bus.tool(CommandId::CHARTS_OPEN_GALLERY).is_some(),
        "the MCP tool stub"
    );
    assert_eq!(
        CommandId::CHARTS_OPEN_GALLERY.tool_name(),
        "app.charts_open_gallery"
    );
}
