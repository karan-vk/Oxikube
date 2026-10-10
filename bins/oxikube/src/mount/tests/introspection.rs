//! The real app's bus, listed the way the palette will (E11-S01): every registered command is in
//! `CommandBus::all()` with a tool stub, and `list` answers per view, selection and read-only
//! flag over the commands the app really registers (not test handlers).

use gpui::TestAppContext;
use oxikube_app::command_bus::{CommandContext, Selection};
use oxikube_domain::Capabilities;
use oxikube_domain::command::{self, CommandId, ViewContext};
use oxikube_domain::ids::Gvk;
use oxikube_testkit::TestPorts;

use super::App;
use crate::app_state::AppState;

#[gpui::test]
fn the_apps_bus_lists_what_runs_where(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.press("enter");
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the mount set the bus").clone();

    // Everything registered is listed once, in the declared registry, with a tool stub (the
    // read-only toggle is privileged: no tool, ADR 0012).
    assert!(bus.all().len() > 40, "the app registers its commands");
    for info in bus.all() {
        assert!(command::lookup(info.id()).is_some(), "{}", info.id());
        assert_eq!(info.has_tool(), bus.tool(info.id()).is_some());
        assert_eq!(info.has_tool(), !info.meta().privileged, "{}", info.id());
    }

    let session = |view| {
        let mut ctx = CommandContext::new(view).with_capabilities(Capabilities::all());
        ctx.cluster_active = true;
        ctx
    };
    let ids = |ctx: &CommandContext| -> Vec<CommandId> {
        bus.list(ctx).iter().map(|info| info.id()).collect()
    };
    let pod = Selection::one(Gvk::new("", "v1", "Pod"));

    // No cluster yet (the catalog): connecting is offered, a namespace pick is not.
    let catalog = ids(&CommandContext::new(ViewContext::Catalog));
    assert!(catalog.contains(&CommandId::CLUSTER_CONNECT));
    assert!(!catalog.contains(&CommandId::NAMESPACE_SELECT));

    // A resource table with a pod selected, writable then read-only.
    let table = session(ViewContext::Table).selecting(pod);
    let writable = ids(&table);
    assert!(writable.contains(&CommandId::POD_VIEW_LOGS));
    assert!(writable.contains(&CommandId::RESOURCE_DELETE));
    assert!(writable.contains(&CommandId::NAMESPACE_SELECT));
    assert!(!writable.contains(&CommandId::LOGS_FIND), "log view only");
    let read_only = ids(&table.read_only(true));
    assert!(read_only.contains(&CommandId::POD_VIEW_LOGS));
    assert!(!read_only.contains(&CommandId::RESOURCE_DELETE));

    // The log viewer's own commands.
    let logs = ids(&session(ViewContext::Logs));
    assert!(logs.contains(&CommandId::LOGS_FIND));
    assert!(!logs.contains(&CommandId::POD_VIEW_LOGS));
}
