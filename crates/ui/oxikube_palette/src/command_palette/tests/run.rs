//! Running a command from the palette: what is dispatched, recents, focus, and the commands that
//! need an operand.

use gpui::{Modifiers, TestAppContext};
use oxikube_app::RecentsStore as _;
use oxikube_domain::command::{Command, CommandId};

use super::{Fixture, pod};

#[gpui::test]
fn typing_and_confirming_dispatches_the_command(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &[]);
    f.open();
    f.type_text("zoom in");
    assert_eq!(f.selected(), Some(CommandId::VIEW_ZOOM_IN));
    f.keys("enter");
    assert!(!f.is_open(), "confirming closes the palette");
    assert_eq!(f.dispatched.commands(), [Command::ViewZoomIn]);
}

#[gpui::test]
fn a_command_on_objects_runs_on_the_selection_the_palette_opened_on(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &["web", "api"]);
    f.open();
    f.type_text("pod view logs");
    assert_eq!(f.selected(), Some(CommandId::POD_VIEW_LOGS));
    // The user changes the selection while the palette is open: the command still acts on what
    // they opened it on.
    let surface = f.surface.clone();
    f.vcx.update(|_, cx| {
        surface.update(cx, |surface, _| surface.target.targets = vec![pod("other")]);
    });
    f.keys("enter");
    let targets: Vec<_> = f
        .dispatched
        .commands()
        .into_iter()
        .map(|command| match command {
            Command::PodViewLogs { target, .. } => target.name.to_string(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(targets, ["web", "api"], "one per selected pod");
}

#[gpui::test]
fn recents_come_first_after_a_dispatch(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &["web"]);
    f.open();
    assert_ne!(f.listed()[0], CommandId::VIEW_ZOOM_OUT);
    f.type_text("zoom out");
    f.keys("enter");
    assert_eq!(f.recents.recent(), [CommandId::VIEW_ZOOM_OUT]);

    f.open();
    let listed = f.listed();
    assert_eq!(
        listed[0],
        CommandId::VIEW_ZOOM_OUT,
        "the recent command heads the list"
    );
    // The rest keep their category order.
    let rest: Vec<_> = listed[1..].to_vec();
    assert!(!rest.contains(&CommandId::VIEW_ZOOM_OUT));

    // A second recent goes above the first.
    f.type_text("reset zoom");
    f.keys("enter");
    f.open();
    assert_eq!(
        &f.listed()[..2],
        [CommandId::VIEW_ZOOM_RESET, CommandId::VIEW_ZOOM_OUT]
    );
}

#[gpui::test]
fn a_typed_query_ranks_by_match_before_recency(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &["web"]);
    f.recents.record(CommandId::VIEW_ZOOM_IN);
    f.open();
    f.type_text("zoom out");
    assert_eq!(
        f.selected(),
        Some(CommandId::VIEW_ZOOM_OUT),
        "a better match beats a recent one"
    );
}

#[gpui::test]
fn the_previous_view_has_the_focus_after_confirm(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &["web"]);
    assert!(f.item_has_focus());
    f.open();
    assert!(!f.item_has_focus(), "the palette's field has it");
    f.type_text("zoom in");
    f.keys("enter");
    assert!(f.item_has_focus(), "the table regains the focus");
    assert_eq!(
        f.dispatched.focused_at_dispatch(),
        [true],
        "the command is sent once the view has the focus back, so it acts on that view"
    );
}

#[gpui::test]
fn escape_runs_nothing_and_gives_the_focus_back(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &["web"]);
    f.open();
    f.type_text("zoom in");
    f.keys("escape");
    assert!(!f.is_open());
    assert!(f.dispatched.commands().is_empty());
    assert!(f.recents.recent().is_empty());
    assert!(f.item_has_focus());
}

#[gpui::test]
fn clicking_a_row_runs_it(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &["web"]);
    f.open();
    f.type_text("zoom in");
    let at = f
        .vcx
        .debug_bounds("palette-command-0")
        .expect("drawn")
        .center();
    f.vcx.simulate_click(at, Modifiers::none());
    f.settle();
    assert_eq!(f.dispatched.ids(), [CommandId::VIEW_ZOOM_IN]);
}

#[gpui::test]
fn a_command_that_needs_an_operand_says_so_and_dispatches_nothing(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &["web"]);
    f.open();
    f.type_text("scale");
    assert_eq!(f.selected(), Some(CommandId::WORKLOAD_SCALE));
    f.keys("enter");
    assert!(
        f.dispatched.commands().is_empty(),
        "no replica count to send"
    );
    assert!(
        f.toasts()
            .iter()
            .any(|toast| toast.contains("Scale Workload needs more information")),
        "{:?}",
        f.toasts()
    );
    assert!(!f.is_open());
}

#[gpui::test]
fn toggling_the_palette_from_the_palette_just_closes_it(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &[]);
    f.open();
    f.type_text("toggle command palette");
    assert_eq!(f.selected(), Some(CommandId::PALETTE_TOGGLE));
    f.keys("enter");
    assert!(!f.is_open(), "closed, not reopened");
    assert!(f.dispatched.commands().is_empty());
}
