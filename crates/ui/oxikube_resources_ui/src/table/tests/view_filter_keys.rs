//! Typing in the filter bar is text, never a table action (#555): from the very first key after
//! `/` (the key focuses the bar at once instead of after the `table::FocusFilter` command's round
//! trip through the bus), and in an inactive window (the table's `Editing` flag is read from the
//! focus when it renders, not from the bar's focus events, which an inactive window never sends).
//! Escape still clears the filter and returns to the rows.

use gpui::{Entity, Focusable as _, KeyContext, TestAppContext};
use oxikube_domain::command::Command;
use oxikube_domain::ids::Gvk;
use oxikube_workspace::CommandDispatcher as _;

use super::fixture::{Fixture, cluster};
use super::p;
use crate::table::ResourceTable;

/// The pods table with the exec actions and its first row selected, so every row key (`a`, `s`,
/// `d`, enter, `j`) would act on something if it ran.
fn open(cx: &mut TestAppContext) -> (Fixture, Entity<ResourceTable>) {
    let mut f = Fixture::with_exec(cx);
    f.connect_with([p("x", "apple-1", "1"), p("x", "web-1", "1")]);
    let table = f.open_pods();
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    assert_eq!(f.selected(&table), ["apple-1"]);
    f.dispatcher.clear();
    (f, table)
}

fn bar_text(f: &mut Fixture, table: &Entity<ResourceTable>) -> String {
    f.vcx
        .update(|_, cx| table.read(cx).filter().read(cx).text().to_owned())
}

fn bar_focused(f: &mut Fixture, table: &Entity<ResourceTable>) -> bool {
    f.vcx.update(|window, cx| {
        let bar = table.read(cx).filter().read(cx);
        bar.focus_handle(cx).contains_focused(window, cx)
    })
}

fn table_focused(f: &mut Fixture, table: &Entity<ResourceTable>) -> bool {
    f.vcx
        .update(|window, cx| table.read(cx).focus_handle(cx).is_focused(window))
}

/// The table's entry in the key context stack of the focused element, as the keymap sees it.
fn table_context(f: &mut Fixture) -> KeyContext {
    f.vcx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window
            .context_stack()
            .into_iter()
            .find(|context| context.contains("Table"))
            .expect("the focus is inside the table")
    })
}

/// What the table sent besides `table::FocusFilter`: any row action (shell, attach, delete,
/// open) would be here.
fn actions_sent(f: &Fixture) -> Vec<Command> {
    f.dispatcher
        .sent()
        .into_iter()
        .filter(|c| !matches!(c, Command::TableFocusFilter { .. }))
        .collect()
}

#[gpui::test]
fn every_key_typed_right_after_slash_is_filter_text(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    // One burst: no frame and no bus round trip between `/` and the text (a fast typist, or the
    // keys queued while the UI thread was busy). `a` is attach, `s` shell, `d` / `p` / `e` are
    // nothing in a table but text in the bar, `j` / `k` move.
    f.keys(&table, "/ a s j k d e p 1 2 0");
    assert_eq!(bar_text(&mut f, &table), "asjkdep120");
    assert!(bar_focused(&mut f, &table), "the bar kept the focus");
    assert_eq!(actions_sent(&f), [], "no table action ran");
    assert!(
        table_context(&mut f).contains("Editing"),
        "the table's key context says Editing while the filter has the focus"
    );
}

#[gpui::test]
fn slash_then_apple_filters_instead_of_attaching(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    f.keys(&table, "/ a p p l e");
    assert_eq!(bar_text(&mut f, &table), "apple");
    assert_eq!(f.names(&table), ["apple-1"]);
    assert_eq!(actions_sent(&f), []);
    assert_eq!(
        f.dispatcher.sent().len(),
        1,
        "`/` still runs `table::FocusFilter`, once: {:?}",
        f.dispatcher.sent()
    );
}

#[gpui::test]
fn the_key_context_says_editing_in_an_inactive_window(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    // An inactive window gets no focus events (the headless audit's setup), so the flag must not
    // depend on them.
    f.vcx.deactivate_window();
    assert!(!table_context(&mut f).contains("Editing"), "on the rows");
    f.keys(&table, "/");
    assert!(bar_focused(&mut f, &table));
    assert!(table_context(&mut f).contains("Editing"));
    f.vcx.simulate_keystrokes("a s");
    f.settle();
    assert_eq!(bar_text(&mut f, &table), "as");
    assert_eq!(actions_sent(&f), [], "neither attach nor shell ran");

    // Escape clears the filter and returns to the rows, where bare keys act again.
    f.vcx.simulate_keystrokes("escape");
    f.settle();
    assert_eq!(bar_text(&mut f, &table), "");
    assert!(table_focused(&mut f, &table));
    assert!(!table_context(&mut f).contains("Editing"));
    // (`as` matched nothing, which dropped the selection.)
    assert_eq!(f.selected(&table), [] as [&str; 0]);
    f.vcx.simulate_keystrokes("j");
    f.settle();
    assert_eq!(
        f.selected(&table),
        ["apple-1"],
        "`j` moves the cursor again"
    );
}

#[gpui::test]
fn escape_right_after_typing_clears_and_returns_to_the_rows(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    f.keys(&table, "/ w e b escape");
    assert_eq!(bar_text(&mut f, &table), "");
    assert_eq!(f.names(&table), ["apple-1", "web-1"], "the filter is gone");
    assert!(
        table_focused(&mut f, &table),
        "the command's echo did not refocus the bar"
    );
    assert_eq!(actions_sent(&f), []);
}

#[gpui::test]
fn a_quick_enter_after_slash_stays_on_the_rows(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    f.keys(&table, "/ w e b enter");
    assert_eq!(bar_text(&mut f, &table), "web");
    assert_eq!(f.names(&table), ["web-1"]);
    assert!(
        table_focused(&mut f, &table),
        "the command's echo did not refocus the bar"
    );
    assert_eq!(actions_sent(&f), [], "enter in the bar opens nothing");
}

#[gpui::test]
fn the_command_alone_still_focuses_the_bar(cx: &mut TestAppContext) {
    // The palette or an agent: `table::FocusFilter` without the key.
    let (mut f, table) = open(cx);
    let dispatcher = f.dispatcher.clone();
    f.vcx.update(|_, cx| {
        dispatcher.dispatch(
            Command::TableFocusFilter {
                cluster: cluster(),
                gvk: Gvk::new("", "v1", "Pod"),
            },
            cx,
        )
    });
    f.settle();
    assert!(bar_focused(&mut f, &table));
    f.vcx.simulate_keystrokes("a");
    f.settle();
    assert_eq!(bar_text(&mut f, &table), "a");
    assert_eq!(actions_sent(&f), []);
}
