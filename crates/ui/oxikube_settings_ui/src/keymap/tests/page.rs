//! The page in a workspace window over a real keymap.

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use oxikube_keymap::{KeymapLayer, RowStatus};

use super::Harness;
use crate::keymap::{ContextFilter, EmptyState};

const USER: &str = r#"[
  { "bindings": { "ctrl-alt-shift-k": "keymap::OpenUser" } },
  { "context": "ResourceTable", "bindings": { "ctrl-alt-shift-j": "keymap::ShowEditor" } }
]"#;

#[gpui::test]
fn the_table_lists_the_default_user_and_unbound_rows(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, USER);
    assert!(h.is_laid_out("keymap-editor"));
    assert!(h.is_laid_out("keymap-table"));
    assert!(h.is_laid_out("keymap-search"));
    assert!(h.is_laid_out("keymap-context"));
    let (total, user_rows, unbound) = h.read(|view, cx| {
        view.table().read(cx, |d| {
            let m = d.model();
            let rows: Vec<_> = (0..m.len()).filter_map(|i| m.row(i)).collect();
            (
                m.total(),
                rows.iter()
                    .filter(|r| r.source == Some(KeymapLayer::User))
                    .count(),
                rows.iter()
                    .filter(|r| r.status == RowStatus::Unbound)
                    .count(),
            )
        })
    });
    assert!(
        total > 50,
        "the shipped keymap has dozens of bindings: {total}"
    );
    assert_eq!(user_rows, 2);
    assert!(unbound > 0, "commands nothing is bound to are listed");
    assert!(!h.is_laid_out("keymap-no-user"), "the user has a keymap");
}

#[gpui::test]
fn the_table_is_virtualised(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "");
    let (total, visible) = h.read(|view, cx| {
        (
            view.table().read(cx, |d| d.model().len()),
            view.table().visible_rows(cx).len(),
        )
    });
    assert!(
        visible > 0 && visible < total,
        "{visible} of {total} rows drawn"
    );
}

#[gpui::test]
fn searching_filters_by_keystroke_spelled_either_way(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, USER);
    h.vcx.update(|_, cx| {
        h.view
            .update(cx, |view, cx| view.set_query("Ctrl+Alt+Shift+K", cx));
    });
    h.settle();
    assert_eq!(h.shown(), ["keymap::OpenUser [ctrl-alt-shift-k]"]);
    h.vcx.update(|_, cx| {
        h.view
            .update(cx, |view, cx| view.set_query("zzzz-no-such", cx));
    });
    h.settle();
    assert!(h.shown().is_empty());
    assert_eq!(
        h.read(|v, cx| v.table().read(cx, |d| d.model().empty_state())),
        EmptyState::NoMatches
    );
    assert!(h.is_laid_out("keymap-empty"), "the empty state is drawn");
}

#[gpui::test]
fn typing_in_the_search_input_filters(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, USER);
    let input = h.read(|view, _| view.search.clone());
    h.vcx
        .update(|window, cx| input.update(cx, |input, cx| input.focus(window, cx)));
    h.vcx.simulate_input("open user keymap");
    h.settle();
    let shown = h.shown();
    assert!(
        !shown.is_empty() && shown.iter().all(|s| s.starts_with("keymap::OpenUser")),
        "{shown:?}"
    );
}

#[gpui::test]
fn the_context_filter_and_the_unbound_switch_narrow_the_list(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, USER);
    h.vcx.update(|_, cx| {
        h.view.update(cx, |view, cx| {
            view.set_context(ContextFilter::Named("ResourceTable".into()), cx);
        });
    });
    h.settle();
    assert!(
        h.shown()
            .contains(&"keymap::ShowEditor [ctrl-alt-shift-j]".to_owned())
    );
    assert!(
        !h.shown()
            .contains(&"keymap::OpenUser [ctrl-alt-shift-k]".to_owned())
    );

    h.vcx.update(|_, cx| {
        h.view.update(cx, |view, cx| {
            view.set_context(ContextFilter::All, cx);
            view.set_show_unbound(false, cx);
        });
    });
    h.settle();
    let unbound = h.read(|v, cx| {
        v.table().read(cx, |d| {
            let m = d.model();
            (0..m.len())
                .filter_map(|i| m.row(i))
                .filter(|r| r.status == RowStatus::Unbound)
                .count()
        })
    });
    assert_eq!(unbound, 0);
}

#[gpui::test]
fn a_hot_reload_updates_the_rows_and_keeps_search_selection_and_scroll(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, USER);
    h.vcx.update(|_, cx| {
        h.view.update(cx, |view, cx| view.set_query("keymap::", cx));
    });
    h.settle();
    // Select the second row showing.
    h.vcx
        .update(|_, cx| h.view.update(cx, |view, cx| view.select(1, cx)));
    let selected = h.read(|v, cx| {
        v.table().read(cx, |d| {
            d.selected()
                .map(|r| (r.action.clone(), r.keystrokes_text()))
        })
    });
    assert!(selected.is_some());
    let before = h.shown().len();

    // The file changes on disk: a new binding appears.
    h.vcx.update(|_, cx| {
        oxikube_keymap::reload_user_keymap(
            cx,
            r#"[{ "bindings": { "ctrl-alt-shift-k": "keymap::OpenUser", "ctrl-alt-shift-e": "keymap::ShowEditor" } }]"#,
        );
    });
    h.settle();
    let shown = h.shown();
    assert!(
        shown.contains(&"keymap::ShowEditor [ctrl-alt-shift-e]".to_owned()),
        "{shown:?}"
    );
    assert!(
        shown.len() > before - 1,
        "rows were replaced, not appended to"
    );
    assert_eq!(
        h.read(|v, cx| v.table().read(cx, |d| d.model().query_text().to_owned())),
        "keymap::"
    );
    let after = h.read(|v, cx| {
        v.table().read(cx, |d| {
            d.selected()
                .map(|r| (r.action.clone(), r.keystrokes_text()))
        })
    });
    assert_eq!(after, selected, "the selection follows its row");
}

#[gpui::test]
fn a_replaced_default_is_marked_and_a_null_is_unbound_by_you(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "");
    // Find a default binding to replace and one to unbind.
    let defaults: Vec<(String, Option<String>, String)> = h.read(|v, cx| {
        v.table().read(cx, |d| {
            let m = d.model();
            (0..m.len())
                .filter_map(|i| m.row(i))
                .filter(|r| r.source == Some(KeymapLayer::Default) && r.status == RowStatus::Active)
                .map(|r| (r.action.clone(), r.context.clone(), r.keystrokes_text()))
                .collect()
        })
    });
    let (_, ctx_a, keys_a) = defaults[0].clone();
    let (_, ctx_b, keys_b) = defaults
        .iter()
        .find(|(_, c, k)| (c, k) != (&ctx_a, &keys_a))
        .cloned()
        .expect("two defaults");
    let section = |ctx: &Option<String>, keys: &str, to: &str| match ctx {
        Some(ctx) => format!(r#"{{"context": "{ctx}", "bindings": {{"{keys}": {to}}}}}"#),
        None => format!(r#"{{"bindings": {{"{keys}": {to}}}}}"#),
    };
    let text = format!(
        "[{}, {}]",
        section(&ctx_a, &keys_a, r#""keymap::OpenUser""#),
        section(&ctx_b, &keys_b, "null"),
    );
    h.vcx
        .update(|_, cx| oxikube_keymap::reload_user_keymap(cx, &text));
    h.settle();
    let statuses: Vec<RowStatus> = h.read(|v, cx| {
        v.table().read(cx, |d| {
            let m = d.model();
            (0..m.len())
                .filter_map(|i| m.row(i))
                .filter(|r| r.source == Some(KeymapLayer::Default))
                .filter(|r| {
                    (&r.context, r.keystrokes_text()) == (&ctx_a, keys_a.clone())
                        || (&r.context, r.keystrokes_text()) == (&ctx_b, keys_b.clone())
                })
                .map(|r| r.status)
                .collect()
        })
    });
    assert!(statuses.contains(&RowStatus::Overridden), "{statuses:?}");
    assert!(statuses.contains(&RowStatus::UnboundByUser), "{statuses:?}");
}

#[gpui::test]
fn open_json_sends_the_open_user_keymap_command(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, USER);
    h.click("keymap-open-json");
    assert_eq!(h.bus.sent(), [Command::KeymapOpenUser]);
}

#[gpui::test]
fn without_a_user_keymap_the_page_points_at_the_file(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "");
    assert!(h.is_laid_out("keymap-no-user"));
}

#[gpui::test]
fn the_keys_move_the_selection_and_enter_opens_the_file(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, USER);
    h.vcx
        .update(|_, cx| h.view.update(cx, |view, cx| view.move_selection(1, cx)));
    h.vcx
        .update(|_, cx| h.view.update(cx, |view, cx| view.move_selection(1, cx)));
    assert_eq!(
        h.read(|v, cx| v.table().read(cx, |d| d.selected_position())),
        Some(1)
    );
    h.vcx
        .update(|_, cx| h.view.update(cx, |view, cx| view.move_selection(-5, cx)));
    assert_eq!(
        h.read(|v, cx| v.table().read(cx, |d| d.selected_position())),
        Some(0)
    );
}

#[gpui::test]
fn enter_in_the_search_box_stays_there_and_on_the_list_opens_the_file(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, USER);
    let input = h.read(|view, _| view.search.clone());
    h.vcx
        .update(|window, cx| input.update(cx, |input, cx| input.focus(window, cx)));
    h.vcx.simulate_keystrokes("down enter");
    h.settle();
    assert!(h.bus.sent().is_empty(), "enter in the search box");
    assert_eq!(
        h.read(|v, cx| v.table().read(cx, |d| d.selected_position())),
        Some(0),
        "down from the search box moves into the list"
    );

    let focus = h.read(|view, _| view.focus.clone());
    h.vcx.update(|window, cx| window.focus(&focus, cx));
    h.vcx.simulate_keystrokes("cmd-enter");
    h.settle();
    assert!(h.bus.sent().is_empty(), "a chord is the keymap's");
    h.vcx.simulate_keystrokes("enter");
    h.settle();
    assert_eq!(h.bus.sent(), [Command::KeymapOpenUser]);
}

#[gpui::test]
fn two_thousand_rows_are_virtualised(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "");
    let rows: Vec<_> = (0..2000)
        .map(|n| {
            super::model::row(
                &format!("Command number {n}"),
                &format!("ns::Action{n}"),
                Some("Workspace"),
                &["ctrl-k"],
                Some(KeymapLayer::Default),
                RowStatus::Active,
            )
        })
        .collect();
    let table = h.read(|view, _| view.table().clone());
    h.vcx
        .update(|_, cx| table.update(cx, |d| d.model_mut().set_rows(rows)));
    h.vcx.update(|_, cx| h.view.update(cx, |_, cx| cx.notify()));
    h.settle();
    let (total, drawn) = h.read(|view, cx| {
        (
            view.table().read(cx, |d| d.model().len()),
            view.table().visible_rows(cx).len(),
        )
    });
    assert_eq!(total, 2000);
    assert!(drawn > 0 && drawn < 100, "{drawn} of {total} rows drawn");
}
