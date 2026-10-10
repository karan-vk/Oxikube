//! Scenario C: `base_keymap: "vim"` in the real app's Pods table (E11 done-when 4): `j` / `k` /
//! `g g` / `shift-g` move the cursor, `d d` meets the guard's confirmation instead of deleting,
//! `/` and `:` keep their jobs, and switching the setting rebinds without a restart.

use gpui::TestAppContext;
use oxikube_testkit::TestPorts;

use super::{check, set_user_settings};
use crate::mount::tests::App;

impl App {
    /// The names of the shown table's rows, in row order.
    fn row_order(&mut self) -> Vec<String> {
        self.shown().expect("a table is shown").1
    }

    /// Writes `settings.json` the way a user edit lands (the store notifies its observers).
    fn set_settings(&mut self, json: &str) {
        self.vcx.update(|_, cx| set_user_settings(cx, json));
        self.vcx.run_until_parked();
    }
}

#[gpui::test]
fn j_k_gg_and_shift_g_move_the_cursor_of_the_real_table(cx: &mut TestAppContext) {
    let mut app = App::keyboard(cx, true);
    let rows = app.row_order();
    assert!(rows.len() >= 3, "{rows:?}");
    let last = rows.last().cloned();
    assert_eq!(app.cursor_name(), None, "no row under the cursor yet");

    app.press("j");
    check!(
        app,
        app.cursor_name().as_ref() == rows.first(),
        "j: the first row"
    );
    app.press("j");
    check!(
        app,
        app.cursor_name().as_ref() == rows.get(1),
        "j j: the second"
    );
    app.press("k");
    check!(
        app,
        app.cursor_name().as_ref() == rows.first(),
        "k: back to the first"
    );
    app.press("shift-g");
    check!(app, app.cursor_name() == last, "shift-g: the last row");
    app.press("g g");
    check!(
        app,
        app.cursor_name().as_ref() == rows.first(),
        "g g: the first row"
    );

    // The walk of the story in one burst.
    app.press("j j k g g shift-g");
    check!(
        app,
        app.cursor_name() == last,
        "j j k g g shift-g ends on the last row"
    );
}

#[gpui::test]
fn d_d_opens_the_guards_confirmation_and_deletes_nothing_by_itself(cx: &mut TestAppContext) {
    let mut app = App::keyboard(cx, true);
    let resources = app
        .ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .resources;
    app.press("j j");
    let on = app.cursor().expect("a row under the cursor");

    app.press("d d");
    app.tick();

    check!(
        app,
        app.delete_dialog_open().is_some(),
        "d d opens the same confirmation as the delete key"
    );
    let dialog = app.delete_dialog_open().expect("open");
    let planned = app
        .vcx
        .update(|_, cx| dialog.read(cx).plan().items()[0].target.clone());
    assert_eq!(planned, on, "it plans the row under the cursor");
    check!(
        app,
        resources.mutating_calls().is_empty(),
        "nothing was sent to the cluster: {:?}",
        resources.mutating_calls()
    );
    assert!(
        app.ports.state.audit_log().is_empty(),
        "and nothing is audited yet"
    );

    // Declining leaves the cluster as it was.
    app.press("escape");
    app.tick();
    assert!(app.delete_dialog_open().is_none());
    assert!(resources.mutating_calls().is_empty());
}

#[gpui::test]
fn slash_and_colon_keep_their_jobs_under_vim(cx: &mut TestAppContext) {
    let mut app = App::keyboard(cx, true);
    let table = app.shown_table().expect("the table");

    // `/` focuses the filter; the vim keys are text in it.
    app.press("/");
    app.type_text("ddjjgg");
    let text = app
        .vcx
        .update(|_, cx| table.read(cx).filter().read(cx).text().to_owned());
    check!(
        app,
        text == "ddjjgg",
        "everything typed after `/` is the filter, got {text:?}"
    );
    assert!(
        app.delete_dialog_open().is_none(),
        "d d in the field is text"
    );
    app.press("escape");
    app.tick();

    // `:` opens the jump bar from the table.
    app.focus_table();
    app.press(":");
    check!(
        app,
        app.jump_bar_open().is_some(),
        "`:` opens the jump bar under vim"
    );
    app.press("escape");
    check!(app, app.jump_bar_open().is_none(), "escape closes it");
}

#[gpui::test]
fn switching_base_keymap_in_settings_rebinds_without_a_restart(cx: &mut TestAppContext) {
    let mut app = App::keyboard(cx, false);
    let last = app.row_order().last().cloned();
    app.press("down");
    let first = app.cursor_name();

    app.press("shift-g");
    check!(
        app,
        app.cursor_name() == first,
        "the default keymap has no shift-g"
    );

    app.set_settings(r#"{ "base_keymap": "vim" }"#);
    app.press("shift-g");
    check!(
        app,
        app.cursor_name() == last,
        "after the edit shift-g is the last row"
    );
    app.press("g g");
    check!(app, app.cursor_name() == first, "and g g the first");

    app.set_settings(r#"{ "base_keymap": "default" }"#);
    app.press("shift-g");
    check!(
        app,
        app.cursor_name() == first,
        "switching back unbinds it again"
    );
}
