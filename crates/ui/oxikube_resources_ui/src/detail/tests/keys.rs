//! The drawer's keys (E07-U559): with the drawer focused after Enter on a row, escape closes it
//! and returns to the table, j / k and the arrows step the table's selection and the drawer
//! follows, 1 to 4 switch the tabs. The bindings are the shipped keymap's.

use gpui::{Entity, TestAppContext};
use oxikube_domain::command::Command;

use super::fixture::{Detail, pod_ref, web_pod};
use crate::detail::{DetailTab, DetailView, Mount};
use crate::table::ResourceTable;
use crate::table::tests::p;

/// A window with the pods `a`, `b`, `c` listed and the detail of `b` opened by Enter on its row.
fn drawer_on_b(cx: &mut TestAppContext) -> (Detail, Entity<ResourceTable>) {
    let mut d = Detail::new(cx, ["a", "b", "c"].map(|name| p("shop", name, "1")));
    let table = d.f.open_pods();
    d.f.keys(&table, "j j enter");
    d.settle();
    (d, table)
}

fn shown_name(d: &mut Detail) -> String {
    let view = d.drawer_view().expect("the drawer shows a detail");
    d.read(&view, |v| v.target().name.to_string())
}

fn press(d: &mut Detail, keys: &str) {
    d.f.vcx.simulate_keystrokes(keys);
    d.settle();
}

fn focused(d: &mut Detail, view: &Entity<DetailView>) -> bool {
    d.f.vcx.update(|window, cx| {
        gpui::Focusable::focus_handle(view.read(cx), cx).contains_focused(window, cx)
    })
}

#[gpui::test]
fn enter_moves_the_focus_into_the_drawer_and_j_k_and_the_arrows_step_the_table(
    cx: &mut TestAppContext,
) {
    let (mut d, table) = drawer_on_b(cx);
    let view = d.drawer_view().expect("opened");
    assert_eq!(shown_name(&mut d), "b");
    assert!(focused(&mut d, &view), "Enter focuses the drawer");

    d.f.dispatcher.clear();
    press(&mut d, "j");
    assert_eq!(shown_name(&mut d), "c", "j shows the next object");
    assert_eq!(d.f.selected(&table), ["c"], "and selects its row");
    assert!(matches!(
        d.f.dispatcher.sent().as_slice(),
        [Command::ResourceOpen { target }] if &*target.name == "c"
    ));
    let view = d.drawer_view().expect("still open");
    assert!(focused(&mut d, &view), "the focus stays in the drawer");

    press(&mut d, "j");
    assert_eq!(shown_name(&mut d), "c", "the last row has no next");
    press(&mut d, "k");
    assert_eq!(shown_name(&mut d), "b");
    press(&mut d, "up");
    assert_eq!(shown_name(&mut d), "a");
    assert_eq!(d.f.selected(&table), ["a"]);
    press(&mut d, "down");
    assert_eq!(shown_name(&mut d), "b");
    assert_eq!(d.f.selected(&table), ["b"]);
}

/// Focuses the drawer's view, as Enter on a row does.
fn focus_drawer(d: &mut Detail) {
    let view = d.drawer_view().expect("the drawer shows a detail");
    d.f.vcx.update(|window, cx| {
        let focus = gpui::Focusable::focus_handle(view.read(cx), cx);
        window.focus(&focus, cx);
    });
}

#[gpui::test]
fn j_and_k_step_from_the_object_the_drawer_shows_not_from_the_table_cursor(
    cx: &mut TestAppContext,
) {
    let (mut d, table) = drawer_on_b(cx);
    // The table's cursor goes to `c`, then the drawer shows `a` as an owner link or the palette
    // would open it.
    d.f.vcx
        .update(|_, cx| table.update(cx, |t, cx| t.move_cursor(1, false, cx)));
    assert_eq!(d.f.selected(&table), ["c"]);
    d.open(&pod_ref("a"));
    assert_eq!(shown_name(&mut d), "a");
    focus_drawer(&mut d);

    press(&mut d, "j");
    assert_eq!(
        shown_name(&mut d),
        "b",
        "the object after `a`, not after `c`"
    );
    assert_eq!(d.f.selected(&table), ["b"]);
    press(&mut d, "k");
    press(&mut d, "k");
    assert_eq!(shown_name(&mut d), "a", "the first row has no previous");
}

#[gpui::test]
fn j_and_k_do_nothing_when_the_shown_object_is_not_listed(cx: &mut TestAppContext) {
    let (mut d, table) = drawer_on_b(cx);
    // `zz` is not a row of the table (filtered out, say).
    d.open(&pod_ref("zz"));
    focus_drawer(&mut d);
    d.f.dispatcher.clear();
    press(&mut d, "j k");
    assert_eq!(shown_name(&mut d), "zz");
    assert!(d.f.dispatcher.sent().is_empty(), "nothing was opened");
    assert_eq!(
        d.f.selected(&table),
        ["b"],
        "the table's cursor did not move"
    );
}

#[gpui::test]
fn number_keys_switch_the_tabs(cx: &mut TestAppContext) {
    let (mut d, _table) = drawer_on_b(cx);
    let view = d.drawer_view().expect("opened");
    for (keys, tab) in [
        ("2", DetailTab::Yaml),
        ("3", DetailTab::Describe),
        ("4", DetailTab::Events),
        ("1", DetailTab::Overview),
    ] {
        press(&mut d, keys);
        assert_eq!(d.read(&view, |v| v.tab()), tab, "{keys}");
    }
    press(&mut d, "5");
    assert_eq!(
        d.read(&view, |v| v.tab()),
        DetailTab::Overview,
        "a pod has no fifth tab"
    );
    press(&mut d, "3");
    assert_eq!(shown_name(&mut d), "b");
}

#[gpui::test]
fn escape_closes_the_drawer_and_returns_to_the_table(cx: &mut TestAppContext) {
    let (mut d, table) = drawer_on_b(cx);
    press(&mut d, "escape");
    assert!(d.drawer_view().is_none(), "the drawer let go of its detail");
    let workspace = d.workspace();
    let open = d.f.vcx.update(|_, cx| {
        workspace
            .read(cx)
            .dock(oxikube_workspace::DockPosition::Right, cx)
            .map(|dock| dock.is_open())
    });
    assert_eq!(open, Some(false), "the dock closed");
    let in_table = d.f.vcx.update(|window, cx| {
        gpui::Focusable::focus_handle(table.read(cx), cx).contains_focused(window, cx)
    });
    assert!(in_table, "the table has the focus again");
    assert_eq!(d.f.selected(&table), ["b"], "its selection is untouched");

    // The table's own keys work at once: j selects the next row.
    press(&mut d, "j");
    assert_eq!(d.f.selected(&table), ["c"]);
}

#[gpui::test]
fn a_pinned_tab_ignores_escape_and_stepping(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    let view = d.open(&pod_ref("web-0"));
    d.click("detail-pin");
    assert_eq!(d.read(&view, |v| v.mount()), Mount::Tab);
    d.f.vcx.update(|window, cx| {
        let focus = gpui::Focusable::focus_handle(view.read(cx), cx);
        window.focus(&focus, cx);
    });
    d.f.dispatcher.clear();
    press(&mut d, "escape j k");
    assert!(d.f.dispatcher.sent().is_empty(), "nothing was opened");
    let workspace = d.workspace();
    let tabs =
        d.f.vcx
            .update(|_, cx| workspace.read(cx).items_of_type::<DetailView>().len());
    assert_eq!(tabs, 1, "the tab is still there");
    press(&mut d, "2");
    assert_eq!(
        d.read(&view, |v| v.tab()),
        DetailTab::Yaml,
        "its tabs still switch"
    );
}
