//! Opening the palette and what it lists.

use gpui::TestAppContext;
use oxikube_domain::command::CommandId;

use super::{Fixture, OPEN_KEY};

#[gpui::test]
fn the_shortcut_opens_and_closes_the_palette(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &["web"]);
    assert!(!f.is_open());
    f.open();
    assert!(f.is_open(), "{OPEN_KEY} opens it");
    f.keys(OPEN_KEY);
    assert!(!f.is_open(), "{OPEN_KEY} again closes it");
    f.open();
    f.keys("escape");
    assert!(!f.is_open(), "escape closes it");
}

#[gpui::test]
fn the_palette_has_the_palette_key_context_around_the_picker(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &[]);
    f.open();
    let stack = f.vcx.update(|window, _| {
        window
            .context_stack()
            .iter()
            .filter_map(|context| context.primary().map(|entry| entry.key.to_string()))
            .collect::<Vec<_>>()
    });
    let palette = stack.iter().position(|name| name == "Palette");
    let picker = stack.iter().position(|name| name == "Picker");
    let input = stack.iter().position(|name| name == "Input");
    assert!(
        palette.is_some(),
        "Palette is in the key context: {stack:?}"
    );
    assert!(palette < picker && picker < input, "{stack:?}");
}

#[gpui::test]
fn the_query_field_has_the_focus_and_takes_printable_keys(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &[]);
    f.open();
    assert!(!f.item_has_focus());
    f.type_text("zoom");
    let query = f.vcx.update(|_, cx| {
        let palette = f.workspace.read(cx).modal_layer().read(cx);
        palette
            .active_modal::<super::CommandPalette>()
            .map(|palette| palette.read(cx).picker().read(cx).query(cx))
    });
    assert_eq!(query.as_deref(), Some("zoom"));
    let listed = f.listed();
    assert!(listed.contains(&CommandId::VIEW_ZOOM_IN));
    assert!(!listed.contains(&CommandId::APP_QUIT));
}

#[gpui::test]
fn it_lists_the_commands_that_can_run_here_by_category_and_title(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &["web"]);
    f.open();
    let listed = f.listed();
    // A table with a pod selected on a writable cluster.
    assert!(listed.contains(&CommandId::POD_VIEW_LOGS));
    assert!(listed.contains(&CommandId::RESOURCE_DELETE));
    assert!(listed.contains(&CommandId::VIEW_ZOOM_IN));
    assert!(listed.contains(&CommandId::PALETTE_TOGGLE));
    // Other views' commands are not offered.
    assert!(!listed.contains(&CommandId::LOGS_FIND), "log view only");
    // Display order: by category, then title.
    let bus = super::declared_index();
    let infos: Vec<_> = listed.iter().map(|id| bus.get(*id).unwrap()).collect();
    let keys: Vec<_> = infos.iter().map(|i| (i.category(), i.title())).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "category, then title");
}

#[gpui::test]
fn each_row_is_drawn_with_its_selector(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &["web"]);
    f.open();
    assert!(f.vcx.debug_bounds("palette-command-0").is_some());
    assert!(f.vcx.debug_bounds("palette-footer").is_some());
    assert!(f.vcx.debug_bounds("palette-show-all").is_some());
}

#[gpui::test]
fn opening_twice_reads_the_selection_again(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &[]);
    f.open();
    assert!(
        !f.listed().contains(&CommandId::POD_VIEW_LOGS),
        "nothing selected"
    );
    f.keys("escape");
    // The user selects a pod, then opens the palette again.
    let surface = f.surface.clone();
    f.vcx.update(|_, cx| {
        surface.update(cx, |surface, _| {
            surface.target.targets = vec![super::pod("web")];
        });
    });
    f.open();
    assert!(f.listed().contains(&CommandId::POD_VIEW_LOGS));
}
