//! The filter bar in a table (E07-S04): `/` focuses it through the command, `escape` clears,
//! `enter` returns, the count, parse errors, the debounce, label selectors that re-key the feed,
//! and namespace scoping.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{Entity, TestAppContext};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::session::NamespaceSelection;
use oxikube_testkit::{ResourceCall, pod};

use super::fixture::{Fixture, cluster};
use crate::filter::{DEBOUNCE, FilterBar, FilterBarEvent, SELECTOR_DEBOUNCE};
use crate::table::ResourceTable;

fn labelled(ns: &str, name: &str, app: &str) -> Resource {
    pod().namespace(ns).name(name).label("app", app).build()
}

fn fixtures() -> Vec<Resource> {
    vec![
        labelled("x", "web-1", "web"),
        labelled("x", "web-2", "web"),
        labelled("x", "db-0", "db"),
        labelled("y", "cache-web", "cache"),
        labelled("y", "api", "api"),
    ]
}

fn open(cx: &mut TestAppContext) -> (Fixture, Entity<ResourceTable>) {
    let mut f = Fixture::new(cx);
    f.connect_with(fixtures());
    let table = f.open_pods();
    (f, table)
}

/// Types `keys` with the bar focused (through the table's own `/`).
fn type_in_bar(f: &mut Fixture, table: &Entity<ResourceTable>, keys: &str) {
    focus_bar(f, table);
    f.vcx.simulate_keystrokes(keys);
    f.settle();
}

fn focus_bar(f: &mut Fixture, table: &Entity<ResourceTable>) {
    f.keys(table, "/");
    assert!(editing(f, table), "`/` put the focus in the filter bar");
}

fn bar(f: &mut Fixture, table: &Entity<ResourceTable>) -> Entity<FilterBar> {
    f.vcx.update(|_, cx| table.read(cx).filter().clone())
}

fn editing(f: &mut Fixture, table: &Entity<ResourceTable>) -> bool {
    let bar = bar(f, table);
    f.vcx.update(|window, cx| {
        let focus = gpui::Focusable::focus_handle(bar.read(cx), cx);
        focus.contains_focused(window, cx)
    })
}

fn table_focused(f: &mut Fixture, table: &Entity<ResourceTable>) -> bool {
    f.vcx
        .update(|window, cx| gpui::Focusable::focus_handle(table.read(cx), cx).is_focused(window))
}

fn count_label(f: &mut Fixture, table: &Entity<ResourceTable>) -> Option<String> {
    let bar = bar(f, table);
    f.vcx.update(|_, cx| bar.read(cx).count_label())
}

fn bar_events(f: &mut Fixture, table: &Entity<ResourceTable>) -> Rc<RefCell<Vec<FilterBarEvent>>> {
    let log = Rc::new(RefCell::new(Vec::new()));
    let sink = log.clone();
    let bar = bar(f, table);
    let subscription = f.vcx.update(|_, cx| {
        cx.subscribe(&bar, move |_, event: &FilterBarEvent, _| {
            sink.borrow_mut().push(event.clone());
        })
    });
    std::mem::forget(subscription);
    log
}

fn changes(log: &Rc<RefCell<Vec<FilterBarEvent>>>) -> usize {
    log.borrow()
        .iter()
        .filter(|e| matches!(e, FilterBarEvent::Changed { .. }))
        .count()
}

fn watched(f: &Fixture) -> Vec<(Option<String>, Option<String>)> {
    f.ports()
        .resources
        .recorded_calls()
        .into_iter()
        .filter_map(|c| match c {
            ResourceCall::Watch {
                namespace, options, ..
            } => Some((namespace, options.label_selector)),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn slash_focuses_the_bar_through_the_command_and_typing_filters(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    assert_eq!(f.names(&table).len(), 5);
    f.dispatcher.clear();
    f.keys(&table, "/");
    assert!(matches!(
        f.dispatcher.sent().as_slice(),
        [Command::TableFocusFilter { cluster: c, gvk }] if *c == cluster() && &*gvk.kind == "Pod"
    ));
    assert!(editing(&mut f, &table), "the command focused the bar");
    let flag = bar(&mut f, &table);
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    f.settle();
    assert!(
        f.vcx.update(|_, cx| flag.read(cx).is_editing()),
        "the bar knows it has the focus, so the table's key context says Editing"
    );

    f.vcx.simulate_keystrokes("w e b");
    f.settle();
    assert_eq!(f.names(&table), ["web-1", "web-2", "cache-web"]);
    // Bare keys are text while the bar has the focus: `j` did not move the cursor.
    f.vcx.simulate_keystrokes("j");
    f.settle();
    assert!(f.selected(&table).is_empty());
    assert_eq!(f.names(&table), [] as [&str; 0], "`webj` matches nothing");
}

#[gpui::test]
fn bare_keys_are_text_while_typing_a_filter(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    focus_bar(&mut f, &table);
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    f.settle();
    f.dispatcher.clear();
    // k9s habit: a leading `/`, and `j` / `k` inside the text. Enter must not open a row.
    f.vcx.simulate_keystrokes("/ w e b - j enter");
    f.settle();
    let bar = bar(&mut f, &table);
    assert_eq!(
        f.vcx.update(|_, cx| bar.read(cx).text().to_owned()),
        "/web-j"
    );
    assert!(
        f.dispatcher.sent().is_empty(),
        "no command ran for those keys: {:?}",
        f.dispatcher.sent()
    );
    assert!(f.selected(&table).is_empty());
}

#[gpui::test]
fn escape_clears_the_filter_and_returns_to_the_rows(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    type_in_bar(&mut f, &table, "d b");
    assert_eq!(f.names(&table), ["db-0"]);
    f.vcx.simulate_keystrokes("escape");
    f.settle();
    assert_eq!(f.names(&table).len(), 5, "the filter is gone");
    assert!(table_focused(&mut f, &table), "the rows have the focus");
    assert!(!editing(&mut f, &table));
    let bar = bar(&mut f, &table);
    assert_eq!(f.vcx.update(|_, cx| bar.read(cx).text().to_owned()), "");
}

#[gpui::test]
fn enter_returns_to_the_rows_without_clearing(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    type_in_bar(&mut f, &table, "w e b");
    f.vcx.simulate_keystrokes("enter");
    f.settle();
    assert_eq!(f.names(&table), ["web-1", "web-2", "cache-web"]);
    assert!(table_focused(&mut f, &table));
    let bar = bar(&mut f, &table);
    assert_eq!(f.vcx.update(|_, cx| bar.read(cx).text().to_owned()), "web");
    // Back on the rows, bare keys move the cursor again.
    f.vcx.simulate_keystrokes("j");
    f.settle();
    assert_eq!(f.selected(&table), ["web-1"]);
}

#[gpui::test]
fn the_count_reads_shown_of_total(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    assert_eq!(count_label(&mut f, &table), None, "no filter, no count");
    type_in_bar(&mut f, &table, "w e b");
    assert_eq!(count_label(&mut f, &table).as_deref(), Some("3 of 5"));
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(f.vcx.debug_bounds("resource-filter-count").is_some());
    f.vcx.simulate_keystrokes("escape");
    f.settle();
    assert_eq!(count_label(&mut f, &table), None, "cleared: no count");
}

#[gpui::test]
fn an_invalid_regex_shows_an_error_and_keeps_the_rows(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    type_in_bar(&mut f, &table, "w e b");
    assert_eq!(f.names(&table).len(), 3);
    f.vcx.simulate_keystrokes("(");
    f.settle();
    let bar = bar(&mut f, &table);
    let error = f
        .vcx
        .update(|_, cx| bar.read(cx).error().map(ToString::to_string));
    assert!(
        error
            .as_deref()
            .is_some_and(|e| e.contains("invalid regex")),
        "{error:?}"
    );
    assert_eq!(
        f.names(&table).len(),
        3,
        "the rows of the last good filter stay"
    );
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(f.vcx.debug_bounds("resource-filter-error").is_some());

    // Fixing it clears the error and applies.
    f.vcx.simulate_keystrokes("backspace");
    f.settle();
    let error = f.vcx.update(|_, cx| bar.read(cx).error().cloned());
    assert_eq!(error, None);
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(f.vcx.debug_bounds("resource-filter-error").is_none());
}

#[gpui::test]
fn the_first_keystroke_applies_at_once_and_a_burst_collapses(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    focus_bar(&mut f, &table);
    let log = bar_events(&mut f, &table);
    // Three keystrokes inside one debounce window.
    f.vcx.simulate_keystrokes("w e b");
    assert_eq!(changes(&log), 1, "the first keystroke went out at once");
    f.vcx.executor().advance_clock(DEBOUNCE * 2);
    f.vcx.run_until_parked();
    assert_eq!(changes(&log), 2, "the other two collapsed into one");
    f.settle();
    assert_eq!(f.names(&table), ["web-1", "web-2", "cache-web"]);
}

#[gpui::test]
fn a_label_selector_rekeys_the_feed_after_a_pause_and_the_server_filters(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    focus_bar(&mut f, &table);
    let log = bar_events(&mut f, &table);
    assert_eq!(watched(&f), [(None, None)]);
    f.vcx.simulate_keystrokes("- l space a p p = w e b");
    f.vcx.run_until_parked();
    assert_eq!(
        changes(&log),
        0,
        "a selector waits for a pause: no watch per keystroke"
    );
    assert_eq!(watched(&f).len(), 1);
    f.vcx.executor().advance_clock(SELECTOR_DEBOUNCE);
    f.settle();
    assert_eq!(
        f.names(&table),
        ["web-1", "web-2"],
        "only the server's matches"
    );
    assert_eq!(
        watched(&f),
        [(None, None), (None, Some("app=web".to_owned()))],
        "re-keyed with the selector"
    );
    assert_eq!(count_label(&mut f, &table).as_deref(), Some("2 of 2"));

    // Enter applies a selector at once.
    f.vcx.simulate_keystrokes("backspace backspace backspace");
    f.vcx.simulate_keystrokes("d b enter");
    f.settle();
    assert_eq!(f.names(&table), ["db-0"]);
    assert!(
        watched(&f)
            .iter()
            .any(|(_, s)| s.as_deref() == Some("app=db"))
    );
}

#[gpui::test]
fn the_filter_composes_with_the_namespace_selection(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    f.sessions
        .set_namespace_selection(&cluster(), NamespaceSelection::single("x"))
        .expect("open session");
    f.settle();
    assert_eq!(f.names(&table), ["db-0", "web-1", "web-2"]);

    type_in_bar(&mut f, &table, "w e b");
    assert_eq!(
        f.names(&table),
        ["web-1", "web-2"],
        "only x: cache-web is in y"
    );
    f.vcx.simulate_keystrokes("escape");
    f.settle();
    focus_bar(&mut f, &table);
    f.vcx.simulate_keystrokes("- l space a p p = w e b enter");
    f.settle();
    assert_eq!(f.names(&table), ["web-1", "web-2"]);
    assert!(
        watched(&f)
            .iter()
            .skip(1)
            .all(|(ns, _)| ns.as_deref() == Some("x")),
        "{:?}: no watch widened past the namespace",
        watched(&f)
    );
    assert_eq!(count_label(&mut f, &table).as_deref(), Some("2 of 2"));
}

#[gpui::test]
fn a_fuzzy_filter_ranks_unless_a_column_is_sorted(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with([
        labelled("x", "a-x-b-x-c", "t"),
        labelled("x", "abc-pod", "t"),
        labelled("x", "a-b-c", "t"),
        labelled("x", "other", "t"),
    ]);
    let table = f.open_pods();
    type_in_bar(&mut f, &table, "- f space a b c enter");
    assert_eq!(
        f.names(&table),
        ["abc-pod", "a-b-c", "a-x-b-x-c"],
        "best match first"
    );
    f.update(&table, |t, cx| {
        t.sort_by(Some((oxikube_app::ColumnId::new("name"), false)), cx)
    });
    assert_eq!(
        f.names(&table),
        ["a-b-c", "a-x-b-x-c", "abc-pod"],
        "a chosen column wins; the fuzzy filter still filters"
    );
}

#[gpui::test]
fn nothing_matching_says_so(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    type_in_bar(&mut f, &table, "z z z");
    assert!(f.names(&table).is_empty());
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(f.vcx.debug_bounds("resource-table-state").is_some());
    let state = f
        .vcx
        .update(|_, cx| table.read(cx).read_rows(cx, |d| d.table_state()));
    assert_eq!(
        state,
        crate::table::states::TableState::FilteredEmpty {
            filter: "zzz".into()
        },
        "the state names the filter, not the cluster"
    );
}
