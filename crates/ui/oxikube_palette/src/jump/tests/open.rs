//! Opening and closing the bar: the `:` key in a table, the bus command, the key context, focus.

use gpui::TestAppContext;

use super::Fixture;

#[gpui::test]
fn colon_in_a_table_opens_the_bar_with_the_caret_in_the_query(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    assert!(f.bar().is_none());
    f.keys(":");
    assert!(f.bar().is_some(), "`:` in a table opens the jump bar");
    assert!(!f.table_has_focus(), "the query field has the focus");
    f.type_text("pods");
    assert_eq!(
        f.query(),
        "pods",
        "the first character was not eaten by the key"
    );
}

#[gpui::test]
fn the_bus_command_opens_it_and_a_second_one_closes_it(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.open();
    assert!(f.bar().is_some());
    f.open();
    assert!(f.bar().is_none(), "toggled off");
    assert!(
        f.table_has_focus(),
        "the view it opened over has the focus back"
    );
}

#[gpui::test]
fn the_key_context_nests_jump_bar_picker_input(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.open();
    let stack = f.vcx.update(|window, _| {
        window
            .context_stack()
            .iter()
            .filter_map(|c| c.primary().map(|entry| entry.key.to_string()))
            .collect::<Vec<_>>()
    });
    let position = |name: &str| stack.iter().position(|c| c == name);
    let (jump, picker, input) = (position("JumpBar"), position("Picker"), position("Input"));
    assert!(jump.is_some(), "{stack:?}");
    assert!(jump < picker && picker < input, "{stack:?}");
}

#[gpui::test]
fn colon_inside_the_bar_is_text(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.keys(":");
    f.type_text("pods ");
    f.keys(":");
    assert!(f.bar().is_some(), "typing `:` did not toggle the bar");
    assert_eq!(f.query(), "pods :");
}

#[gpui::test]
fn escape_closes_it_without_running_anything(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.keys(":");
    f.type_text("deploy kube-system");
    f.keys("escape");
    assert!(f.bar().is_none());
    assert!(f.table_has_focus(), "focus went back to the table");
    assert_eq!(f.take_sent(), []);
}

#[gpui::test]
fn it_opens_listing_the_aliases_of_the_shown_cluster(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.open();
    let listed = f.read(|d| d.completions());
    for word in ["pods", "po", "deploy", "ctx", "ns", "q"] {
        assert!(listed.iter().any(|w| w == word), "{word} is offered");
    }
    assert!(
        f.vcx.debug_bounds("jump-hints").is_some(),
        "the key hints are drawn"
    );
}
