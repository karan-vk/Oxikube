//! Typing filters the list, the keyboard moves through it, only the visible rows are built.

use super::{Fixture, Where};
use crate::help::Row;
use crate::picker::PickerDelegate as _;

#[gpui::test]
fn typing_filters_by_title_category_and_keystroke(cx: &mut gpui::TestAppContext) {
    let mut f = Fixture::new(cx, Where::Table);
    f.keys("?");
    let all = f.listing().entry_count();
    assert!(all > 10, "a table has many keys: {all}");

    f.type_text("yaml");
    let listing = f.listing();
    assert!(listing.has("y", "resource_table::ViewYaml"));
    assert!(listing.entry_count() < all);

    // By keystroke: `ctrl-d` is delete.
    f.set_query("ctrl-d");
    assert!(f.listing().has("ctrl-d", "resource_table::DeleteSelected"));

    // By category label.
    f.set_query("pod");
    let listing = f.listing();
    assert!(
        listing.headers.iter().any(|h| h == "Pod"),
        "{:?}",
        listing.headers
    );

    // Nothing matches: an explanatory line, not a blank modal.
    f.set_query("qqqqqq");
    assert!(f.vcx.debug_bounds("picker-empty").is_some());
    assert_eq!(f.listing().entry_count(), 0);
}

#[gpui::test]
fn the_keyboard_walks_the_entries_and_skips_the_headers(cx: &mut gpui::TestAppContext) {
    let mut f = Fixture::new(cx, Where::Table);
    f.keys("?");
    let picker = f.picker();
    let selected = |f: &mut Fixture| {
        let picker = picker.clone();
        f.vcx.update(|_, cx| {
            let d = &picker.read(cx).delegate;
            (d.selected_index(), d.rows()[d.selected_index()].clone())
        })
    };
    let (first, row) = selected(&mut f);
    assert!(
        matches!(row, Row::Entry { .. }),
        "the first entry, not a header"
    );
    assert_eq!(first, 1);

    let rows = f.listing().rows.len();
    let mut seen_headers = 0;
    for _ in 0..rows {
        f.keys("down");
        let (_, row) = selected(&mut f);
        match row {
            Row::Entry { .. } => {}
            Row::Header { .. } => seen_headers += 1,
        }
    }
    assert_eq!(seen_headers, 0, "a header is never selected");
    // Wraps around: after `rows` presses of down over the entries we are somewhere valid, and
    // `up` from the first entry goes to the last.
    f.keys("home");
    let (home, _) = selected(&mut f);
    assert_eq!(home, 1);
    f.keys("up");
    let (last, row) = selected(&mut f);
    assert!(matches!(row, Row::Entry { .. }));
    assert_eq!(last, rows - 1);
}

#[gpui::test]
fn enter_closes_it_and_runs_nothing(cx: &mut gpui::TestAppContext) {
    let mut f = Fixture::new(cx, Where::Table);
    f.keys("?");
    f.keys("enter");
    assert!(f.overlay().is_none());
    assert!(f.focused_is_probe());
}

#[gpui::test]
fn only_the_visible_rows_are_built(cx: &mut gpui::TestAppContext) {
    let mut f = Fixture::new(cx, Where::Table);
    // Three hundred bindings of one context: more than any screen shows.
    let overlay_rows = (0..300)
        .map(|i| format!("ctrl-alt-{i}"))
        .collect::<Vec<_>>();
    let bindings = overlay_rows
        .iter()
        .map(|key| format!("\"{key}\": \"resource_table::ViewYaml\""))
        .collect::<Vec<_>>()
        .join(", ");
    let user =
        format!(r#"[{{"context": "ResourceTable && !Editing", "bindings": {{{bindings}}}}}]"#);
    f.vcx.update(|_, cx| {
        oxikube_keymap::init_with_text(&user, Default::default(), cx);
    });
    f.keys("?");
    let total = f.listing().rows.len();
    assert!(total > 300, "{total} rows");
    let built = (0..total)
        .filter(|ix| {
            let selector: &'static str = Box::leak(format!("picker-row-{ix}").into_boxed_str());
            f.vcx.debug_bounds(selector).is_some()
        })
        .count();
    assert!(
        built > 0 && built < 40,
        "{built} of {total} rows were built"
    );
}
