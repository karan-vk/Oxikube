//! Unavailable commands: hidden by default, listed marked with the reason under "Show all", and
//! never run from the palette.

use gpui::TestAppContext;
use oxikube_domain::command::CommandId;

use super::{Fixture, SHOW_ALL_KEY};

#[gpui::test]
fn a_read_only_session_hides_the_mutations(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &["web"]);
    f.open();
    assert!(f.listed().contains(&CommandId::RESOURCE_DELETE));
    f.keys("escape");

    f.env.set_read_only(true);
    f.open();
    let listed = f.listed();
    assert!(
        !listed.contains(&CommandId::RESOURCE_DELETE),
        "hidden, not greyed"
    );
    assert!(listed.contains(&CommandId::POD_VIEW_LOGS), "reads stay");
    assert!(!f.vcx.update(|_, cx| palette_show_all(&f.workspace, cx)));
}

fn palette_show_all(
    workspace: &gpui::Entity<oxikube_workspace::Workspace>,
    cx: &gpui::App,
) -> bool {
    workspace
        .read(cx)
        .modal_layer()
        .read(cx)
        .active_modal::<crate::command_palette::CommandPalette>()
        .is_some_and(|palette| palette.read(cx).picker().read(cx).delegate.show_all())
}

#[gpui::test]
fn show_all_lists_them_with_the_reason_and_the_shortcut_toggles_it(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &["web"]);
    f.env.set_read_only(true);
    f.open();
    let hidden = f.listed().len();
    f.keys(SHOW_ALL_KEY);
    let listed = f.listed();
    assert!(listed.len() > hidden, "show all adds the unavailable");
    assert!(listed.contains(&CommandId::RESOURCE_DELETE));
    assert!(f.vcx.update(|_, cx| palette_show_all(&f.workspace, cx)));

    // The unavailable row says why.
    let ix = listed
        .iter()
        .position(|id| *id == CommandId::RESOURCE_DELETE)
        .unwrap();
    let reason = f.vcx.update(|_, cx| {
        let palette = f.workspace.read(cx).modal_layer().read(cx);
        let palette = palette
            .active_modal::<crate::command_palette::CommandPalette>()
            .unwrap();
        let picker = palette.read(cx).picker().read(cx);
        picker
            .delegate
            .row(ix)
            .and_then(|(_, row)| row.unavailable)
            .map(|r| r.to_string())
    });
    assert_eq!(reason.as_deref(), Some("This cluster is read-only"));

    f.keys(SHOW_ALL_KEY);
    assert_eq!(f.listed().len(), hidden, "toggled back");
}

#[gpui::test]
fn the_footer_toggle_is_clickable(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &["web"]);
    f.env.set_read_only(true);
    f.open();
    let hidden = f.listed().len();
    let at = f
        .vcx
        .debug_bounds("palette-show-all")
        .expect("drawn")
        .center();
    f.vcx.simulate_click(at, gpui::Modifiers::none());
    f.settle();
    assert!(f.listed().len() > hidden);
}

#[gpui::test]
fn confirming_an_unavailable_command_does_nothing(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &["web"]);
    f.env.set_read_only(true);
    f.open();
    f.keys(SHOW_ALL_KEY);
    f.type_text("delete");
    // Select the unavailable `resource::Delete` row.
    let listed = f.listed();
    let ix = listed
        .iter()
        .position(|id| *id == CommandId::RESOURCE_DELETE)
        .expect("listed");
    for _ in 0..ix {
        f.keys("down");
    }
    assert_eq!(f.selected(), Some(CommandId::RESOURCE_DELETE));
    f.keys("enter");
    assert!(f.is_open(), "the palette stays");
    assert!(f.dispatched.commands().is_empty(), "nothing was dispatched");
    assert!(
        f.recents.recent().is_empty(),
        "and it is not a recent command"
    );
}
