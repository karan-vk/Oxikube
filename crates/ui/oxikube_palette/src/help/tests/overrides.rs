//! Overrides are marked: the user's `keymap.json`, the vim base, a default the user unbound.

use gpui::TestAppContext;
use oxikube_keymap::KeymapOptions;

use super::{Fixture, Where};
use crate::help::{HelpSource, HelpState};

fn entry_for(f: &mut Fixture, action: &str, keys: &str) -> Option<(HelpSource, HelpState)> {
    f.model()
        .entries()
        .iter()
        .find(|e| e.action == action && e.keystroke_text() == keys)
        .map(|e| (e.source, e.state))
}

#[gpui::test]
fn a_user_rebind_of_y_is_listed_with_the_user_chip(cx: &mut TestAppContext) {
    let user = r#"[{"context": "ResourceTable && !Editing", "bindings": {"y": null, "x": "resource_table::ViewYaml"}}]"#;
    let mut f = Fixture::with_keymap(cx, Where::Table, user, KeymapOptions::default());
    f.keys("?");
    assert_eq!(
        entry_for(&mut f, "resource_table::ViewYaml", "x"),
        Some((HelpSource::User, HelpState::Active))
    );
    // The default key the user nulled is listed too, as unbound.
    assert_eq!(
        entry_for(&mut f, "resource_table::ViewYaml", "y"),
        Some((HelpSource::Default, HelpState::Unbound(HelpSource::User)))
    );
    f.set_query("user");
    assert!(
        f.vcx.debug_bounds("help-chip-user").is_some(),
        "the chip is drawn"
    );
}

#[gpui::test]
fn a_default_the_user_unbound_is_listed_as_unbound(cx: &mut TestAppContext) {
    let user = r#"[{"context": "ResourceTable && !Editing", "bindings": {"d": null}}]"#;
    let mut f = Fixture::with_keymap(cx, Where::Table, user, KeymapOptions::default());
    f.keys("?");
    let found = entry_for(&mut f, "resource_table::ViewDescribe", "d");
    assert_eq!(
        found,
        Some((HelpSource::Default, HelpState::Unbound(HelpSource::User)))
    );
    f.set_query("unbound");
    assert!(f.vcx.debug_bounds("help-chip-unbound").is_some());
}

#[gpui::test]
fn the_vim_base_keymap_shows_the_base_chip(cx: &mut TestAppContext) {
    let options = KeymapOptions {
        vim: true,
        ..KeymapOptions::default()
    };
    let mut f = Fixture::with_keymap(cx, Where::Table, "", options);
    f.keys("?");
    assert_eq!(
        entry_for(&mut f, "table::SelectNext", "j"),
        Some((HelpSource::Base, HelpState::Active))
    );
    // A shipped default has no chip.
    assert_eq!(
        entry_for(&mut f, "resource_table::ViewYaml", "y"),
        Some((HelpSource::Default, HelpState::Active))
    );
    f.set_query("base");
    assert!(f.vcx.debug_bounds("help-chip-base").is_some());
}
