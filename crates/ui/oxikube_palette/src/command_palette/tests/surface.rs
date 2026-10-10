//! A command the focused view runs through its own flow (a table's delete dialog) goes to that
//! view, not to the bus once per object.

use oxikube_app::CommandTarget;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::Gvk;

use super::{Fixture, pod};
use crate::command_palette::Launch;

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
    let mut f = Fixture::declared(cx, &["web"]);
    f.surface_runs(&[CommandId::RESOURCE_DELETE], true);
    pick_delete(&mut f);
    assert_eq!(f.recents.recent(), [CommandId::RESOURCE_DELETE]);
}

#[test]
fn a_launch_for_the_view_builds_no_commands_until_the_view_declines() {
    let target = CommandTarget::none()
        .in_cluster(super::cluster())
        .of_kind(Gvk::new("", "v1", "Pod"))
        .selecting(vec![pod("a"), pod("b")]);
    let launch = Launch::on_surface(CommandId::RESOURCE_DELETE, target);
    assert!(launch.commands.is_empty(), "nothing built at confirm");
    let commands = launch.fallback();
    assert_eq!(
        commands.len(),
        2,
        "one per pod, made when the view declined"
    );
    assert!(matches!(commands[0], Command::ResourceDelete { .. }));
}

#[gpui::test]
fn confirming_a_table_command_over_a_select_all_does_not_stall(cx: &mut gpui::TestAppContext) {
    let names: Vec<String> = (0..10_000).map(|i| format!("pod-{i}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut f = Fixture::declared(cx, &names);
    f.surface_runs(&[CommandId::RESOURCE_DELETE], true);
    f.open();
    f.type_text("resource delete");
    assert_eq!(f.selected(), Some(CommandId::RESOURCE_DELETE));
    let started = std::time::Instant::now();
    f.keys("enter");
    let elapsed = started.elapsed();
    assert_eq!(f.surface_ran().len(), 1, "the table got the selection once");
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "confirm over 10k selected pods took {elapsed:?}"
    );
}
