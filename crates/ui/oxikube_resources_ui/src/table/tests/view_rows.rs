//! The table's rows: virtualisation at 10 000 rows, sorting by any column through the store,
//! header clicks, status colours, empty states.

use std::sync::Arc;

use gpui::TestAppContext;
use oxikube_app::ColumnId;
use oxikube_app::columns::Tone;
use oxikube_domain::Resource;
use oxikube_testkit::pod;

use super::fixture::Fixture;
use crate::table::ToneColors;

fn pod_at(name: &str, restarts: u32, created: &str) -> Resource {
    pod()
        .namespace("x")
        .name(name)
        .restarts(restarts)
        .created(created)
        .build()
}

#[gpui::test]
fn ten_thousand_rows_build_only_the_visible_ones(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with(
        (0..10_000).map(|i| pod().namespace("load").name(format!("pod-{i:05}")).build()),
    );
    let table = f.open_pods();
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let (rows, cells, columns) = f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            (d.rows().len(), d.rendered_cells, d.layout().visible_len())
        })
    });
    assert_eq!(rows, 10_000, "every pod is a row");
    assert!(cells > 0, "the table drew nothing");
    // A test window shows a few dozen rows; 10 000 rows would be 10 000 x columns cells.
    assert!(
        cells < 200 * columns,
        "{cells} cells drawn for {rows} rows: the rows are not virtualised"
    );
    let visible = f
        .vcx
        .update(|_, cx| table.read(cx).table().visible_rows(cx));
    assert!(visible.len() < 200, "visible = {visible:?}");
}

#[gpui::test]
fn sorts_by_name_age_and_a_numeric_column_through_the_store(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with([
        pod_at("Bravo", 10, "2026-01-01T00:00:00Z"),
        pod_at("alpha", 9, "2025-06-01T00:00:00Z"),
        pod_at("charlie", 100, "2026-02-01T00:00:00Z"),
    ]);
    let table = f.open_pods();
    assert_eq!(
        f.names(&table),
        ["Bravo", "alpha", "charlie"],
        "kubectl order by default"
    );

    f.update(&table, |t, cx| {
        t.sort_by(Some((ColumnId::new("name"), false)), cx)
    });
    assert_eq!(
        f.names(&table),
        ["alpha", "Bravo", "charlie"],
        "names ignore case"
    );

    f.update(&table, |t, cx| {
        t.sort_by(Some((ColumnId::new("restarts"), false)), cx)
    });
    assert_eq!(
        f.names(&table),
        ["alpha", "Bravo", "charlie"],
        "9 < 10 < 100"
    );
    f.update(&table, |t, cx| {
        t.sort_by(Some((ColumnId::new("restarts"), true)), cx)
    });
    assert_eq!(f.names(&table), ["charlie", "Bravo", "alpha"]);

    f.update(&table, |t, cx| {
        t.sort_by(Some((ColumnId::new("age"), false)), cx)
    });
    assert_eq!(
        f.names(&table),
        ["charlie", "Bravo", "alpha"],
        "youngest first"
    );

    f.update(&table, |t, cx| t.sort_by(None, cx));
    assert_eq!(f.names(&table), ["Bravo", "alpha", "charlie"]);
}

#[gpui::test]
fn clicking_a_header_sorts_by_that_column(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with([
        pod_at("a", 10, "2026-01-01T00:00:00Z"),
        pod_at("b", 9, "2026-01-01T00:00:00Z"),
        pod_at("c", 100, "2026-01-01T00:00:00Z"),
    ]);
    let table = f.open_pods();
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let header = f
        .vcx
        .debug_bounds("th-restarts")
        .expect("the restarts header was laid out")
        .center();
    // The library cycles unsorted, descending, ascending.
    f.vcx.simulate_click(header, gpui::Modifiers::none());
    f.settle();
    let sort = f
        .vcx
        .update(|_, cx| table.read(cx).read_rows(cx, |d| d.layout().sort().cloned()));
    assert_eq!(sort, Some((ColumnId::new("restarts"), true)));
    assert_eq!(f.names(&table), ["c", "a", "b"]);
}

#[gpui::test]
fn status_cells_take_the_theme_oxikube_colours(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with([
        pod().namespace("x").name("ok").running().build(),
        pod().namespace("x").name("bad").crash_loop().build(),
        pod().namespace("x").name("wait").pending().build(),
    ]);
    let table = f.open_pods();
    let tones = f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let status = ColumnId::new("status");
            d.rows()
                .iter()
                .map(|row| {
                    d.provider()
                        .cell(row, &status, jiff::Timestamp::now())
                        .tone()
                })
                .collect::<Vec<_>>()
        })
    });
    // Rows are in kubectl order: bad, ok, wait.
    assert_eq!(tones, [Tone::Error, Tone::Ok, Tone::Warn]);

    // The colours come from the active theme's `oxikube` block.
    let mut theme = oxikube_theme::ThemeTokens::fallback(oxikube_theme::Appearance::Dark).clone();
    theme.oxikube.status_running = gpui::red();
    theme.oxikube.status_pending = gpui::yellow();
    theme.oxikube.status_failed = gpui::blue();
    let colors = f.vcx.update(|_, cx| {
        cx.set_global(oxikube_theme::ActiveTheme(Arc::new(theme)));
        ToneColors::current(cx)
    });
    assert_eq!(colors.of(Tone::Ok), gpui::red());
    assert_eq!(colors.of(Tone::Warn), gpui::yellow());
    assert_eq!(colors.of(Tone::Error), gpui::blue());
    let text = f
        .vcx
        .update(|_, cx| oxikube_ui::ActiveTokens::colors(&*cx).text);
    assert_eq!(colors.of(Tone::Neutral), text);
}

#[gpui::test]
fn an_empty_list_says_so(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with([]);
    let table = f.open_pods();
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(f.vcx.debug_bounds("resource-table-state").is_some());
    let (state, title) = f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let state = d.table_state();
            let title = crate::table::states::copy(&state, d.labels()).title;
            (state, title)
        })
    });
    assert_eq!(state, crate::table::TableState::Empty);
    assert_eq!(title, "No pods in all namespaces");
}
