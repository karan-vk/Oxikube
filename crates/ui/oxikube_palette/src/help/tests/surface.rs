//! The dialog's accessibility, the empty and unfocused states.

use gpui::TestAppContext;

use super::{Fixture, Where};
use crate::help::{ACCESSIBLE_NAME, HelpModel, HelpScope};

/// The accessibility tree is only built while assistive technology is attached, which the test
/// platform cannot do; the elements are asked what they would report instead.
fn reports(element: &impl gpui::Element) -> (Option<gpui::Role>, Option<String>) {
    let role = element.a11y_role();
    let mut node = gpui::accesskit::Node::new(role.unwrap_or(gpui::Role::GenericContainer));
    element.write_a11y_info(&mut node);
    (role, node.label().map(str::to_owned))
}

#[gpui::test]
fn the_overlay_is_a_named_dialog(cx: &mut TestAppContext) {
    let frame = crate::help::overlay::frame(gpui::KeyContext::default());
    assert!(
        gpui::Element::id(&frame).is_some(),
        "an id, or it is not exposed"
    );
    assert_eq!(
        reports(&frame),
        (Some(gpui::Role::Dialog), Some(ACCESSIBLE_NAME.to_owned()))
    );
    // The rows: headings for the groups, list items (named with their keys) for the bindings.
    let mut f = Fixture::new(cx, Where::Table);
    f.keys("?");
    let model = f.model();
    f.vcx.update(|_, cx| {
        let heading = crate::help::render::header(model.entries()[0].category, 3, 0, cx);
        let (role, label) = reports(&heading);
        assert_eq!(role, Some(gpui::Role::Heading));
        assert!(label.is_some_and(|l| l.contains("3 keys")));
        let entry = &model.entries()[0];
        let row = crate::help::render::entry(entry, &[], model.scope(), false, 1, cx);
        let (role, label) = reports(&row);
        assert_eq!(role, Some(gpui::Role::ListItem));
        let label = label.expect("named");
        assert!(label.contains(entry.title.as_ref()), "{label}");
        assert!(label.contains(&entry.keystroke_text()), "{label}");
    });
}

#[gpui::test]
fn the_search_field_and_the_list_are_reached_with_the_keyboard_alone(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, Where::Table);
    f.keys("?");
    // The field has the focus on open: typing filters without a click.
    f.type_text("view");
    let picker = f.picker();
    let query = f.vcx.update(|_, cx| picker.read(cx).query(cx));
    assert_eq!(query, "view");
    // Down moves the selection in the list without leaving the field.
    let before = f.selected();
    f.keys("down");
    let after = f.selected();
    assert_ne!(before, after);
}

#[gpui::test]
fn nothing_focused_lists_every_binding(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, Where::Table);
    // Drop the focus: no context is in force, so `?` has no key; the command still works.
    f.vcx.update(|window, cx| window.blur(cx));
    f.settle();
    let stack = f.vcx.update(|window, _| window.context_stack());
    assert!(stack.is_empty(), "nothing is focused");
    let workspace = f.workspace.clone();
    f.vcx.update(|window, cx| {
        let host = crate::help::HelpHost::new(&workspace);
        host.toggle(window, cx);
    });
    f.settle();
    let model = f.model();
    assert_eq!(model.scope(), &HelpScope::Everything);
    let listing = f.listing();
    assert!(listing.has("y", "resource_table::ViewYaml"), "table keys");
    assert!(listing.has("w", "log_view::ToggleWrap"), "log keys");
    assert!(f.vcx.debug_bounds("help-scope").is_some());
}

#[gpui::test]
fn a_context_with_no_bindings_explains_itself(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, Where::Table);
    let workspace = f.workspace.clone();
    f.vcx.update(|window, cx| {
        let model = std::sync::Arc::new(HelpModel::build(
            HelpScope::Focused {
                innermost: "Nowhere".into(),
            },
            Vec::new(),
            Vec::new(),
        ));
        workspace.update(cx, |ws, cx| {
            ws.toggle_modal(window, cx, move |window, cx| {
                crate::help::HelpOverlay::new(model, window, cx)
            });
        });
    });
    f.settle();
    assert!(
        f.vcx.debug_bounds("picker-empty").is_some(),
        "an explanatory message, not a blank modal"
    );
}
