//! A command the focused view runs through its own flow (a table's delete dialog) goes to that
//! view, not to the bus once per object.

use oxikube_domain::command::{Command, CommandId};

use super::{Fixture, pod};

fn pick_delete(f: &mut Fixture) {
    f.open();
    f.type_text("resource delete");
    assert_eq!(f.selected(), Some(CommandId::RESOURCE_DELETE));
    f.keys("enter");
    assert!(!f.is_open());
}

#[gpui::test]
fn delete_on_a_selection_is_one_call_to_the_table_not_a_command_per_pod(
    cx: &mut gpui::TestAppContext,
) {
    let mut f = Fixture::declared(cx, &["web", "api", "db"]);
    f.surface_runs(&[CommandId::RESOURCE_DELETE], true);
    pick_delete(&mut f);

    assert_eq!(
        f.surface_ran(),
        [(
            CommandId::RESOURCE_DELETE,
            vec![pod("web"), pod("api"), pod("db")],
            true
        )],
        "the table is handed the whole selection once, with the focus back"
    );
    assert!(
        f.dispatched.commands().is_empty(),
        "no ResourceDelete per pod reaches the bus: {:?}",
        f.dispatched.commands()
    );
}

#[gpui::test]
fn a_table_that_declines_leaves_the_commands_to_the_bus(cx: &mut gpui::TestAppContext) {
    let mut f = Fixture::declared(cx, &["web", "api"]);
    f.surface_runs(&[CommandId::RESOURCE_DELETE], false);
    pick_delete(&mut f);

    assert_eq!(f.surface_ran().len(), 1, "asked first");
    let sent: Vec<_> = f.dispatched.ids();
    assert_eq!(sent, [CommandId::RESOURCE_DELETE; 2], "then one per pod");
    assert!(matches!(
        f.dispatched.commands()[0],
        Command::ResourceDelete { .. }
    ));
}

#[gpui::test]
fn a_command_the_table_does_not_own_still_goes_to_the_bus(cx: &mut gpui::TestAppContext) {
    let mut f = Fixture::declared(cx, &["web"]);
    f.surface_runs(&[CommandId::RESOURCE_DELETE], true);
    f.open();
    f.type_text("zoom in");
    f.keys("enter");
    assert!(f.surface_ran().is_empty());
    assert_eq!(f.dispatched.commands(), [Command::ViewZoomIn]);
}

#[gpui::test]
fn recents_record_a_command_the_table_ran(cx: &mut gpui::TestAppContext) {
    use oxikube_app::RecentsStore as _;
    let mut f = Fixture::declared(cx, &["web"]);
    f.surface_runs(&[CommandId::RESOURCE_DELETE], true);
    pick_delete(&mut f);
    assert_eq!(f.recents.recent(), [CommandId::RESOURCE_DELETE]);
}
