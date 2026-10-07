//! The context menu, the palette's list and the keys.

use gpui::TestAppContext;
use oxikube_domain::command::{Command, CommandId};
use oxikube_ports::ClusterPrefs;

use super::{picker, pod_with, sent_exec};
use crate::table::tests::fixture::Fixture;
use crate::table::tests::p;

fn labels(f: &mut Fixture, table: &gpui::Entity<crate::table::ResourceTable>) -> Vec<String> {
    f.vcx
        .update(|_, cx| table.read(cx).action_entries(cx))
        .into_iter()
        .map(|entry| entry.label)
        .collect()
}

#[gpui::test]
fn a_pod_offers_shell_attach_and_debug_after_logs_and_a_node_offers_only_its_own_shell(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::with_exec(cx);
    f.connect_with([p("x", "web-0", "1")]);
    let pods = f.open_pods();
    assert_eq!(
        labels(&mut f, &pods),
        ["Logs", "Shell", "Attach", "Debug", "Delete"]
    );

    let nodes = f.open(crate::actions::tests::nodes_kind());
    assert_eq!(
        labels(&mut f, &nodes),
        ["Shell", "Delete"],
        "a node's Shell is `node::Shell`, not a pod's; it has no Attach"
    );
}

#[gpui::test]
fn a_selection_of_several_pods_offers_neither(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    f.connect_with([p("x", "web-0", "1"), p("x", "web-1", "1")]);
    let pods = f.open_pods();
    f.update(&pods, |t, cx| t.select_all(cx));
    assert_eq!(labels(&mut f, &pods), ["Delete 2 objects"]);
}

#[gpui::test]
fn the_menu_item_opens_a_shell_in_the_pods_only_container(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    f.connect_with([pod_with("x", "web-0", &["app"])]);
    let _table = f.open_pods();
    f.dispatcher.clear();
    crate::actions::tests::right_click(&mut f, 0);
    // Open, Copy Name, Select All, Logs, Shell.
    crate::actions::tests::choose(&mut f, 4);
    f.settle();
    let sent = sent_exec(&f);
    assert!(
        matches!(sent.as_slice(), [Command::PodShell { target, container: Some(c) }]
            if &*target.name == "web-0" && c == "app"),
        "{sent:?}"
    );
    assert!(picker(&mut f).is_none(), "one container asks nothing");
}

#[gpui::test]
fn the_keys_s_and_a_open_a_shell_and_attach_for_the_cursor_row(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    f.connect_with([pod_with("x", "web-0", &["app"])]);
    let table = f.open_pods();
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.dispatcher.clear();
    f.keys(&table, "s");
    f.keys(&table, "a");
    let sent = sent_exec(&f);
    assert!(
        matches!(
            sent.as_slice(),
            [Command::PodShell { .. }, Command::PodAttach { .. }]
        ),
        "{sent:?}"
    );
}

#[gpui::test]
fn a_pod_that_cannot_be_read_still_dispatches_and_lets_the_terminal_explain(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::with_exec(cx);
    f.connect_with([pod_with("x", "web-0", &["app", "proxy"])]);
    let table = f.open_pods();
    f.ports()
        .resources
        .script()
        .get
        .push_err(oxikube_domain::OxiError::forbidden(
            "pods \"web-0\" is forbidden",
        ));
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.dispatcher.clear();
    f.keys(&table, "s");
    let sent = sent_exec(&f);
    assert!(
        matches!(
            sent.as_slice(),
            [Command::PodShell {
                container: None,
                ..
            }]
        ),
        "no get on pods must not block exec: {sent:?}"
    );
    assert!(picker(&mut f).is_none());
}

#[gpui::test]
fn a_read_only_cluster_greys_them_out_unless_it_allows_shells(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    f.connect_with([pod_with("x", "web-0", &["app"])]);
    let pods = f.open_pods();
    f.sessions
        .set_read_only(&crate::table::tests::fixture::cluster(), true)
        .unwrap();
    let entries = f.vcx.update(|_, cx| pods.read(cx).action_entries(cx));
    for command in [CommandId::POD_SHELL, CommandId::POD_ATTACH] {
        let entry = entries.iter().find(|e| e.command() == command).unwrap();
        assert!(!entry.is_enabled(), "{command}");
        let reason = entry.reason().unwrap();
        assert!(reason.contains("shells are blocked"), "{reason}");
    }
    // The key says why instead of opening anything.
    f.update(&pods, |t, cx| t.move_cursor(1, false, cx));
    f.dispatcher.clear();
    f.keys(&pods, "s");
    assert!(sent_exec(&f).is_empty());
    let toasts = crate::actions::tests::toasts(&mut f);
    assert!(
        toasts
            .iter()
            .any(|t| t.message.contains("shells are blocked")),
        "{toasts:?}"
    );

    // The cluster's own setting lets them through.
    f.sessions.set_prefs_table(
        oxikube_ports::ClusterPrefsTable::new(ClusterPrefs::default()).with_cluster(
            crate::table::tests::fixture::cluster(),
            ClusterPrefs {
                read_only: true,
                exec_in_read_only: true,
                ..ClusterPrefs::default()
            },
        ),
    );
    f.settle();
    let entries = f.vcx.update(|_, cx| pods.read(cx).action_entries(cx));
    let shell = entries
        .iter()
        .find(|e| e.command() == CommandId::POD_SHELL)
        .unwrap();
    assert!(shell.is_enabled());
    let delete = entries
        .iter()
        .find(|e| e.command() == CommandId::RESOURCE_DELETE)
        .unwrap();
    assert!(
        !delete.is_enabled(),
        "read-only still blocks every mutation"
    );
}
