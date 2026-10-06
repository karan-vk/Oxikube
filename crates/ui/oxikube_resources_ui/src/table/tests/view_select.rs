//! Multi-select with the mouse and the keys, the commands the keys send, and deltas that move
//! the selected rows.

use std::time::Duration;

use gpui::{Modifiers, TestAppContext};
use oxikube_domain::command::Command;
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::Timeline;
use oxikube_ui::table::{RowClick, TableEvent};

use super::fixture::{Fixture, cluster};
use super::p;

fn click(f: &mut Fixture, row: usize, modifiers: Modifiers) {
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let at = f
        .vcx
        .debug_bounds(Box::leak(format!("td-{row}-0").into_boxed_str()))
        .unwrap_or_else(|| panic!("row {row} was not laid out"))
        .center();
    f.vcx.simulate_click(at, modifiers);
    f.settle();
}

fn pods(f: &mut Fixture, names: &[&str]) -> gpui::Entity<crate::table::ResourceTable> {
    f.connect_with(names.iter().map(|n| p("x", n, "1")));
    f.open_pods()
}

#[gpui::test]
fn click_shift_click_and_cmd_click_select_rows(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let table = pods(&mut f, &["a", "b", "c", "d", "e"]);
    click(&mut f, 1, Modifiers::none());
    assert_eq!(f.selected(&table), ["b"]);
    click(&mut f, 3, Modifiers::shift());
    assert_eq!(f.selected(&table), ["b", "c", "d"]);
    click(&mut f, 0, Modifiers::secondary_key());
    assert_eq!(f.selected(&table), ["a", "b", "c", "d"]);
    click(&mut f, 2, Modifiers::secondary_key());
    assert_eq!(f.selected(&table), ["a", "b", "d"]);
    click(&mut f, 4, Modifiers::none());
    assert_eq!(f.selected(&table), ["e"]);
}

#[gpui::test]
fn a_double_click_opens_the_row(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let table = pods(&mut f, &["a", "b"]);
    f.update(&table, |t, cx| {
        t.on_table_event(
            &TableEvent::RowClicked(RowClick {
                row: 1,
                extend: false,
                toggle: false,
                count: 2,
            }),
            cx,
        );
    });
    let opened: Vec<String> = f
        .dispatcher
        .sent()
        .iter()
        .filter_map(|c| match c {
            Command::ResourceOpen { target } => Some(target.name.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(opened, ["b"]);
}

#[gpui::test]
fn keys_move_extend_open_select_all_and_clear(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let table = pods(&mut f, &["a", "b", "c", "d"]);
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = events.clone();
    let _watch = f.vcx.update(|_, cx| {
        cx.subscribe(
            &table,
            move |_, event: &crate::table::ResourceTableEvent, _| {
                sink.borrow_mut().push(event.clone());
            },
        )
    });

    f.keys(&table, "j j");
    assert_eq!(
        f.selected(&table),
        ["b"],
        "the first j lands on the first row"
    );
    f.keys(&table, "down k");
    assert_eq!(f.selected(&table), ["b"]);
    f.keys(&table, "shift-j shift-down");
    assert_eq!(f.selected(&table), ["b", "c", "d"]);

    f.dispatcher.clear();
    f.keys(&table, "enter");
    assert!(matches!(
        f.dispatcher.sent().as_slice(),
        [Command::ResourceOpen { target }] if &*target.name == "d" && target.cluster == cluster()
    ));
    // `resource::Open` reached the table back through the views: the detail drawer's hook.
    assert!(events.borrow().iter().any(|e| matches!(
        e,
        crate::table::ResourceTableEvent::OpenDetail(target) if &*target.name == "d"
    )));

    let select_all = if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    };
    f.keys(&table, select_all);
    assert!(matches!(
        f.dispatcher.sent().last(),
        Some(Command::ResourceSelectAll { .. })
    ));
    assert_eq!(f.selected(&table), ["a", "b", "c", "d"]);

    f.keys(&table, "escape");
    assert!(f.selected(&table).is_empty());
}

#[gpui::test]
fn copy_name_goes_through_the_command_to_the_clipboard(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let table = pods(&mut f, &["a", "b"]);
    f.keys(&table, "j j");
    let copy = if cfg!(target_os = "macos") {
        "cmd-c"
    } else {
        "ctrl-c"
    };
    f.keys(&table, copy);
    assert!(matches!(
        f.dispatcher.sent().last(),
        Some(Command::ResourceCopyName { target }) if &*target.name == "b"
    ));
    let clipboard = f
        .vcx
        .update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(clipboard.as_deref(), Some("b"));
}

#[gpui::test]
fn a_right_click_selects_the_row_it_hits_unless_it_is_selected(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let table = pods(&mut f, &["a", "b", "c"]);
    f.update(&table, |t, cx| t.select_all(cx));
    f.update(&table, |t, cx| {
        t.on_table_event(&TableEvent::RightClickedRow(Some(1)), cx)
    });
    assert_eq!(f.selected(&table), ["a", "b", "c"]);
    f.update(&table, |t, cx| t.clear_selection(cx));
    f.update(&table, |t, cx| {
        t.on_table_event(&TableEvent::RightClickedRow(Some(2)), cx)
    });
    assert_eq!(f.selected(&table), ["c"]);
}

#[gpui::test]
fn deltas_keep_the_selection_by_resource_not_by_index(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let ports = f.ports();
    let timeline = Timeline::new()
        .ok_at(
            Duration::ZERO,
            DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
                p("x", "a", "1"),
                p("x", "b", "1"),
                p("x", "c", "1"),
            ])]),
        )
        .ok_at(
            Duration::from_secs(1),
            DeltaBatch::from_deltas(vec![
                // Lands between a and b: b moves from row 1 to row 2.
                Delta::Applied(p("x", "a2", "1")),
                Delta::Applied(p("x", "b", "2")),
                Delta::Deleted(p("x", "c", "1")),
            ]),
        )
        .keep_open();
    ports.resources.script().watch.push_ok(timeline);
    f.connect_with([]);
    let table = f.open_pods();
    assert_eq!(f.names(&table), ["a", "b", "c"]);
    f.update(&table, |t, cx| {
        t.on_table_event(
            &TableEvent::RowClicked(RowClick {
                row: 1,
                extend: false,
                toggle: false,
                count: 1,
            }),
            cx,
        );
        t.on_table_event(
            &TableEvent::RowClicked(RowClick {
                row: 2,
                extend: false,
                toggle: true,
                count: 1,
            }),
            cx,
        );
    });
    assert_eq!(f.selected(&table), ["b", "c"]);

    ports.resources.clock().advance(Duration::from_secs(1));
    f.settle();
    assert_eq!(f.names(&table), ["a", "a2", "b"]);
    assert_eq!(
        f.selected(&table),
        ["b"],
        "b kept although it moved; c went with its row"
    );
    let selected_row = f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            (0..3)
                .filter(|&i| d.selection().contains_object(&d.rows()[i]))
                .collect::<Vec<_>>()
        })
    });
    assert_eq!(selected_row, [2], "the highlight follows the row");
}

/// Right-clicks row `row`, opens its context menu and lets the feed move the rows (`a2` lands
/// above the clicked `b`) before the menu entry at `entry` is chosen with the keys.
fn menu_entry_after_churn(cx: &mut TestAppContext, entry: usize) -> (Fixture, Vec<Command>) {
    let mut f = Fixture::new(cx);
    let ports = f.ports();
    let timeline = Timeline::new()
        .ok_at(
            Duration::ZERO,
            DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
                p("x", "a", "1"),
                p("x", "b", "1"),
                p("x", "c", "1"),
            ])]),
        )
        .ok_at(
            Duration::from_secs(1),
            DeltaBatch::from_deltas(vec![Delta::Applied(p("x", "a2", "1"))]),
        )
        .keep_open();
    ports.resources.script().watch.push_ok(timeline);
    f.connect_with([]);
    let table = f.open_pods();
    assert_eq!(f.names(&table), ["a", "b", "c"]);

    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let at = f
        .vcx
        .debug_bounds("td-1-0")
        .expect("row 1 was laid out")
        .center();
    f.vcx
        .simulate_mouse_down(at, gpui::MouseButton::Right, Modifiers::none());
    f.vcx
        .simulate_mouse_up(at, gpui::MouseButton::Right, Modifiers::none());
    f.settle();
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(f.selected(&table), ["b"]);

    ports.resources.clock().advance(Duration::from_secs(1));
    f.settle();
    assert_eq!(f.names(&table), ["a", "a2", "b", "c"], "b moved to row 2");

    f.dispatcher.clear();
    let keys = std::iter::repeat_n("down", entry + 1)
        .chain(["enter"])
        .collect::<Vec<_>>()
        .join(" ");
    f.vcx.simulate_keystrokes(&keys);
    f.settle();
    let sent = f.dispatcher.sent();
    (f, sent)
}

#[gpui::test]
fn the_context_menu_opens_the_object_right_clicked_although_its_row_moved(cx: &mut TestAppContext) {
    let (_f, sent) = menu_entry_after_churn(cx, 0);
    let opened: Vec<String> = sent
        .iter()
        .filter_map(|c| match c {
            Command::ResourceOpen { target } => Some(target.name.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(opened, ["b"]);
}

#[gpui::test]
fn the_context_menu_copies_the_name_right_clicked_although_its_row_moved(cx: &mut TestAppContext) {
    let (mut f, sent) = menu_entry_after_churn(cx, 1);
    assert!(
        matches!(sent.as_slice(), [Command::ResourceCopyName { target }] if &*target.name == "b"),
        "{sent:?}"
    );
    let clipboard = f
        .vcx
        .update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(clipboard.as_deref(), Some("b"));
}
