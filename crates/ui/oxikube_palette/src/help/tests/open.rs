//! `?` through the shipped keymap: where it opens, where it is a character, how it closes.

use super::{Fixture, Where};

#[gpui::test]
fn question_mark_in_a_table_lists_the_table_keys(cx: &mut gpui::TestAppContext) {
    let mut f = Fixture::new(cx, Where::Table);
    assert!(f.overlay().is_none());
    f.keys("?");
    assert!(f.overlay().is_some(), "? opened the overlay");
    let listing = f.listing();
    assert!(listing.has("y", "resource_table::ViewYaml"), "k9s y");
    assert!(listing.has("ctrl-d", "resource_table::DeleteSelected"));
    assert!(listing.has("d", "resource_table::ViewDescribe"));
    assert!(
        !listing.has_keys("w"),
        "the log viewer's `w` does not work in a table"
    );
}

#[gpui::test]
fn the_list_differs_with_the_log_view_focused(cx: &mut gpui::TestAppContext) {
    let mut f = Fixture::new(cx, Where::Logs);
    f.keys("?");
    let listing = f.listing();
    assert!(listing.has("w", "log_view::ToggleWrap"));
    assert!(listing.has("s", "log_view::ToggleAutoscroll"));
    assert!(
        !listing.has_keys("y") && !listing.has_keys("ctrl-d"),
        "the table's keys are not in force in the log view"
    );
}

#[gpui::test]
fn in_a_text_field_question_mark_is_a_character(cx: &mut gpui::TestAppContext) {
    let mut f = Fixture::new(cx, Where::TableFilter);
    f.type_text("pod?");
    assert!(f.overlay().is_none(), "the overlay stays closed");
    let typed = f.vcx.update(|_, cx| f.probe.read(cx).typed(cx));
    assert_eq!(typed, "pod?", "the field got the character");
}

#[gpui::test]
fn escape_and_question_mark_both_close_it_and_focus_returns(cx: &mut gpui::TestAppContext) {
    let mut f = Fixture::new(cx, Where::Table);
    assert!(f.focused_is_probe());
    f.keys("?");
    assert!(f.overlay().is_some());
    assert!(!f.focused_is_probe(), "the search field has the focus");

    f.keys("escape");
    assert!(f.overlay().is_none(), "escape closes");
    assert!(f.focused_is_probe(), "focus is back on the view");

    f.keys("?");
    assert!(f.overlay().is_some());
    f.keys("?");
    assert!(
        f.overlay().is_none(),
        "? closes it again while the field is empty"
    );
    assert!(f.focused_is_probe());
}

#[gpui::test]
fn with_text_in_the_search_field_question_mark_is_searched_for(cx: &mut gpui::TestAppContext) {
    let mut f = Fixture::new(cx, Where::Table);
    f.keys("?");
    f.type_text("yaml");
    f.keys("?");
    assert!(f.overlay().is_some(), "still open");
    let picker = f.picker();
    let query = f.vcx.update(|_, cx| picker.read(cx).query(cx));
    assert_eq!(query, "yaml?");
}

#[gpui::test]
fn opening_it_twice_does_not_stack_overlays(cx: &mut gpui::TestAppContext) {
    let mut f = Fixture::new(cx, Where::Table);
    f.keys("?");
    let first = f.overlay().expect("open");
    f.keys("escape ?");
    let second = f.overlay().expect("open again");
    assert_ne!(
        first.entity_id(),
        second.entity_id(),
        "a fresh overlay per open"
    );
}

#[gpui::test]
fn help_never_replaces_another_open_modal(cx: &mut gpui::TestAppContext) {
    use oxikube_workspace::DialogModal;

    let mut f = Fixture::new(cx, Where::Table);
    let workspace = f.workspace.clone();
    f.vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.toggle_modal(window, cx, |_, cx| DialogModal::new("Delete pod?", cx));
        });
    });
    f.settle();
    let dialog_open = |f: &mut Fixture| {
        let workspace = f.workspace.clone();
        f.vcx.update(|_, cx| {
            workspace
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<DialogModal>()
                .is_some()
        })
    };
    assert!(dialog_open(&mut f));

    // The bus door and the key door both end in `toggle`.
    let workspace = f.workspace.clone();
    f.vcx.update(|window, cx| {
        crate::help::HelpHost::new(&workspace).toggle(window, cx);
    });
    f.settle();
    assert!(f.overlay().is_none(), "no overlay over a pending dialog");
    assert!(dialog_open(&mut f), "the dialog is still the open modal");

    // Once the dialog is gone, help opens as usual.
    f.keys("escape");
    assert!(!dialog_open(&mut f));
    let workspace = f.workspace.clone();
    f.vcx.update(|window, cx| {
        crate::help::HelpHost::new(&workspace).toggle(window, cx);
    });
    f.settle();
    assert!(f.overlay().is_some());
}
