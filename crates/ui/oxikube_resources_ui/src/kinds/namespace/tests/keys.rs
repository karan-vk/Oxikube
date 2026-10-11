//! The Namespaces table's keys: `u` use, `p` pin, `0`-`9` All or the nth favourite.

use gpui::TestAppContext;
use oxikube_domain::command::Command;

use super::{Env, ns};
use crate::table::tests::fixture::{Fixture, cluster};
use crate::table::tests::p;

fn sent(env: &Env) -> Vec<Command> {
    env.f.dispatcher.sent()
}

#[gpui::test]
fn u_uses_the_namespace_under_the_cursor(cx: &mut TestAppContext) {
    let mut env = Env::standard(cx);
    env.cursor_to("team-a");
    env.f.dispatcher.clear();
    env.keys("u");
    assert_eq!(
        sent(&env),
        [Command::NamespaceSelect {
            cluster: cluster(),
            namespaces: vec!["team-a".into()],
        }]
    );
}

#[gpui::test]
fn u_with_several_rows_selected_uses_the_cursor_row_only(cx: &mut TestAppContext) {
    let mut env = Env::standard(cx);
    env.cursor_to("team-a");
    env.keys("shift-down");
    assert_eq!(env.f.selected(&env.table).len(), 2);
    env.f.dispatcher.clear();
    env.keys("u");
    let selects: Vec<_> = sent(&env)
        .into_iter()
        .filter(|c| matches!(c, Command::NamespaceSelect { .. }))
        .collect();
    assert_eq!(
        selects.len(),
        1,
        "one namespace is used, not a selection: {selects:?}"
    );
}

#[gpui::test]
fn p_pins_the_namespace_under_the_cursor(cx: &mut TestAppContext) {
    let mut env = Env::standard(cx);
    env.cursor_to("team-b");
    env.f.dispatcher.clear();
    env.keys("p");
    assert_eq!(
        sent(&env),
        [Command::NamespaceToggleFavourite {
            cluster: cluster(),
            namespace: "team-b".into(),
        }]
    );
}

#[gpui::test]
fn the_digits_select_all_or_the_nth_favourite(cx: &mut TestAppContext) {
    let mut env = Env::standard(cx);
    env.f.dispatcher.clear();
    env.keys("0 1 2 9");
    assert_eq!(
        sent(&env),
        [0u8, 1, 2, 9].map(|slot| Command::NamespaceUseFavourite {
            cluster: cluster(),
            slot,
        })
    );
}

#[gpui::test]
fn the_digits_work_in_every_table_but_u_and_p_are_the_namespaces(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    f.ports()
        .resources
        .insert(ns("team-a", serde_json::json!({})));
    f.connect_with([p("team-a", "web-0", "1")]);
    let pods = f.open_pods();
    f.dispatcher.clear();
    f.keys(&pods, "down u p 1");
    assert_eq!(
        f.dispatcher.sent(),
        [Command::NamespaceUseFavourite {
            cluster: cluster(),
            slot: 1,
        }],
        "only the digit does anything on Pods"
    );
}

#[gpui::test]
fn digits_typed_into_the_filter_are_text_not_keys(cx: &mut TestAppContext) {
    let mut env = Env::standard(cx);
    env.f.dispatcher.clear();
    env.keys("/ 1 u p");
    assert!(
        sent(&env).iter().all(|c| !matches!(
            c,
            Command::NamespaceUseFavourite { .. }
                | Command::NamespaceSelect { .. }
                | Command::NamespaceToggleFavourite { .. }
        )),
        "{:?}",
        sent(&env)
    );
}

#[gpui::test]
fn the_all_namespaces_menu_entry_is_slot_zero(cx: &mut TestAppContext) {
    use oxikube_domain::command::CommandId;

    let mut env = Env::standard(cx);
    env.cursor_to("team-a");
    env.f.dispatcher.clear();
    let table = env.table.clone();
    let targets = env.f.vcx.update(|_, cx| table.read(cx).action_targets(cx));
    env.f.vcx.update(|window, cx| {
        table.update(cx, |table, cx| {
            table.run_action(CommandId::NAMESPACE_USE_FAVOURITE, targets, window, cx)
        })
    });
    env.f.settle();
    assert_eq!(
        sent(&env),
        [Command::NamespaceUseFavourite {
            cluster: cluster(),
            slot: 0,
        }]
    );
}
