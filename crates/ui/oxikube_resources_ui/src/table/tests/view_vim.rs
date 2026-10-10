//! The vim base keymap (E11-S09) in a real table: `base_keymap: "vim"` is `set_vim_layer(true)`,
//! the shipped `vim.json` is bound with the shipped defaults, and the keys are pressed with
//! `simulate_keystrokes` into the focused table, so GPUI's pending-keystroke handling (`d` then
//! `j`) is the real one. `oxikube_keymap/tests/vim.rs` resolves the same keys per context.

use gpui::{Entity, TestAppContext};
use oxikube_domain::command::Command;

use super::fixture::Fixture;
use super::p;
use crate::actions::tests::dialog;
use crate::table::ResourceTable;

const NAMES: [&str; 9] = ["a", "b", "c", "d", "e", "f", "g", "h", "i"];

fn open(cx: &mut TestAppContext, vim: bool) -> (Fixture, Entity<ResourceTable>) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with(NAMES.iter().map(|n| p("x", n, "1")));
    let table = f.open_pods();
    if vim {
        f.vcx
            .update(|_, cx| oxikube_keymap::set_vim_layer(cx, true));
    }
    f.keys(&table, "j");
    assert_eq!(f.selected(&table), ["a"]);
    f.dispatcher.clear();
    (f, table)
}

fn cursor(f: &mut Fixture, table: &Entity<ResourceTable>) -> String {
    f.selected(table).first().cloned().expect("a selected row")
}

#[gpui::test]
fn j_k_gg_and_shift_g_walk_the_rows(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx, true);
    f.keys(&table, "j j");
    assert_eq!(cursor(&mut f, &table), "c");
    f.keys(&table, "k");
    assert_eq!(cursor(&mut f, &table), "b");
    f.keys(&table, "shift-g");
    assert_eq!(cursor(&mut f, &table), "i", "G is the last row");
    f.keys(&table, "g g");
    assert_eq!(cursor(&mut f, &table), "a", "gg is the first row");
    // The walk of the story in one burst.
    f.keys(&table, "j j k g g shift-g");
    assert_eq!(cursor(&mut f, &table), "i");
    // A lone `g` waits; the next key that does not complete it is its own key.
    f.keys(&table, "g j");
    assert_eq!(
        cursor(&mut f, &table),
        "i",
        "j at the end stays; g alone moved nothing"
    );
}

#[gpui::test]
fn ctrl_d_and_ctrl_u_move_half_a_page(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx, true);
    // Nine rows: a page is eight rows, half is four.
    f.keys(&table, "ctrl-d");
    assert_eq!(cursor(&mut f, &table), "e");
    f.keys(&table, "ctrl-d");
    assert_eq!(cursor(&mut f, &table), "i");
    f.keys(&table, "ctrl-u");
    assert_eq!(cursor(&mut f, &table), "e");
    f.keys(&table, "ctrl-u ctrl-u");
    assert_eq!(cursor(&mut f, &table), "a");
    assert!(
        dialog(&mut f).is_none(),
        "ctrl-d scrolls in vim; it is not k9s's delete"
    );
}

#[gpui::test]
fn d_d_opens_the_delete_dialog_and_deletes_nothing_by_itself(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx, true);
    f.keys(&table, "j d d");
    assert_eq!(cursor(&mut f, &table), "b");
    let dialog = dialog(&mut f).expect("dd opens the same confirmation as delete");
    assert!(
        f.ports().resources.mutating_calls().is_empty(),
        "never deletes by itself"
    );
    drop(dialog);
    assert!(
        f.dispatcher.sent().is_empty(),
        "no describe either: {:?}",
        f.dispatcher.sent()
    );
}

#[gpui::test]
fn d_then_j_is_just_j(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx, true);
    f.keys(&table, "d j");
    assert!(dialog(&mut f).is_none(), "d then j does not delete");
    assert_eq!(cursor(&mut f, &table), "b", "the j moved the cursor");
    assert!(f.ports().resources.mutating_calls().is_empty());
    assert!(
        f.dispatcher.sent().is_empty(),
        "and the d did not describe: {:?}",
        f.dispatcher.sent()
    );
    // The same for y and g.
    f.keys(&table, "y j");
    assert_eq!(cursor(&mut f, &table), "c");
    assert!(f.dispatcher.sent().is_empty(), "y j copied nothing");
}

#[gpui::test]
fn y_y_copies_the_name_of_the_cursor_row(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx, true);
    f.keys(&table, "j j y y");
    let sent = f.dispatcher.sent();
    assert!(
        matches!(sent.as_slice(), [Command::ResourceCopyName { target }] if &*target.name == "c"),
        "{sent:?}"
    );
    let clipboard = f
        .vcx
        .update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(clipboard.as_deref(), Some("c"));
}

#[gpui::test]
fn g_d_and_g_y_keep_describe_and_yaml_reachable(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx, true);
    f.keys(&table, "g d");
    f.keys(&table, "g y");
    let sent = f.dispatcher.sent();
    assert!(
        matches!(
            sent.as_slice(),
            [
                Command::ResourceViewDescribe { .. },
                Command::ResourceViewYaml { .. }
            ]
        ),
        "{sent:?}"
    );
}

#[gpui::test]
fn slash_focuses_the_filter_and_the_vim_keys_are_text_in_it(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx, true);
    f.keys(&table, "/ d d j k g g y y");
    let text = f
        .vcx
        .update(|_, cx| table.read(cx).filter().read(cx).text().to_owned());
    assert_eq!(text, "ddjkggyy", "everything typed after `/` is the filter");
    assert!(
        dialog(&mut f).is_none(),
        "dd in the field is text, not delete"
    );
    assert!(
        f.dispatcher
            .sent()
            .iter()
            .all(|c| matches!(c, Command::TableFocusFilter { .. })),
        "only `/` ran a command: {:?}",
        f.dispatcher.sent()
    );
    // Escape returns to the rows, where vim keys act again.
    f.vcx.simulate_keystrokes("escape");
    f.settle();
    assert!(f.ports().resources.mutating_calls().is_empty());
}

#[gpui::test]
fn with_the_default_base_the_vim_keys_do_nothing_and_ctrl_d_deletes(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx, false);
    f.keys(&table, "g g");
    assert_eq!(cursor(&mut f, &table), "a");
    f.keys(&table, "shift-g");
    assert_eq!(
        cursor(&mut f, &table),
        "a",
        "G moves nothing without the vim base"
    );
    f.keys(&table, "ctrl-u y y");
    assert!(
        f.dispatcher
            .sent()
            .iter()
            .all(|c| !matches!(c, Command::ResourceCopyName { .. }))
    );
    // k9s's ctrl-d is the delete dialog; `d d` is describe twice, never a delete.
    f.keys(&table, "d d");
    assert!(dialog(&mut f).is_none());
    f.keys(&table, "ctrl-d");
    assert!(dialog(&mut f).is_some());
}
