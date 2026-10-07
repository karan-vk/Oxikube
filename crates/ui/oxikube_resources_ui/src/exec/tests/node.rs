//! "Shell" on a node's row, key and detail header (E09-S09): `node::Shell`, offered where a node
//! is, greyed out on every read-only cluster, and never opening anything by itself (the bus asks
//! for the confirmation).

use gpui::TestAppContext;
use oxikube_domain::command::{Command, CommandId};
use oxikube_ports::{ClusterPrefs, ClusterPrefsTable};
use oxikube_testkit::node;

use super::sent_node_shell;
use crate::actions::tests::nodes_kind;
use crate::detail::tests::fixture::Detail;
use crate::table::tests::fixture::{Fixture, cluster};

fn node_ref(name: &str) -> oxikube_domain::ids::ResourceRef {
    oxikube_domain::ids::ResourceRef::cluster_scoped(
        cluster(),
        oxikube_domain::ids::Gvk::new("", "v1", "Node"),
        name,
    )
}

fn with_node(cx: &mut TestAppContext) -> (Fixture, gpui::Entity<crate::table::ResourceTable>) {
    let mut f = Fixture::with_exec(cx);
    let ports = f.ports();
    ports.discovery.set_kinds([nodes_kind()]);
    ports.resources.insert(node().name("worker-1").build());
    f.connect_with([]);
    let table = f.open(nodes_kind());
    (f, table)
}

fn set_read_only(f: &mut Fixture, exec_in_read_only: bool) {
    f.sessions.set_prefs_table(
        ClusterPrefsTable::new(ClusterPrefs::default()).with_cluster(
            cluster(),
            ClusterPrefs {
                read_only: true,
                exec_in_read_only,
                ..ClusterPrefs::default()
            },
        ),
    );
    f.settle();
}

#[gpui::test]
fn the_key_s_on_a_node_sends_node_shell_for_the_cursor_row(cx: &mut TestAppContext) {
    let (mut f, table) = with_node(cx);
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.dispatcher.clear();
    f.keys(&table, "s");
    assert_eq!(
        sent_node_shell(&f),
        [Command::NodeShell {
            target: node_ref("worker-1")
        }]
    );
    assert!(
        f.dispatcher
            .sent()
            .iter()
            .all(|command| !matches!(command, Command::PodShell { .. })),
        "a node is not a pod"
    );
}

#[gpui::test]
fn the_menu_item_sends_node_shell(cx: &mut TestAppContext) {
    let (mut f, _table) = with_node(cx);
    f.dispatcher.clear();
    crate::actions::tests::right_click(&mut f, 0);
    // Open, Copy Name, Select All, Shell.
    crate::actions::tests::choose(&mut f, 3);
    assert_eq!(sent_node_shell(&f).len(), 1, "{:?}", f.dispatcher.sent());
}

#[gpui::test]
fn a_read_only_cluster_greys_the_node_shell_out_even_when_pod_shells_are_allowed(
    cx: &mut TestAppContext,
) {
    let (mut f, table) = with_node(cx);
    set_read_only(&mut f, true);
    let entries = f.vcx.update(|_, cx| table.read(cx).action_entries(cx));
    let shell = entries
        .iter()
        .find(|e| e.command() == CommandId::NODE_SHELL)
        .expect("offered");
    assert!(!shell.is_enabled(), "it creates a pod: a mutation");
    let reason = shell.reason().expect("a reason");
    assert!(reason.contains("read-only"), "{reason}");
    assert!(!reason.contains("shells are blocked"), "{reason}");

    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.dispatcher.clear();
    f.keys(&table, "s");
    assert!(sent_node_shell(&f).is_empty(), "nothing is sent");
    let toasts = crate::actions::tests::toasts(&mut f);
    assert!(
        toasts.iter().any(|t| t.message.contains("read-only")),
        "{toasts:?}"
    );
}

#[gpui::test]
fn a_nodes_header_has_a_shell_button_that_dispatches_node_shell(cx: &mut TestAppContext) {
    let mut d = Detail::with_exec(cx, [node().name("worker-1").build()]);
    d.open(&node_ref("worker-1"));
    assert!(d.shown("detail-node-shell"));
    assert!(
        !d.shown("detail-shell") && !d.shown("detail-attach"),
        "the pod buttons are for pods"
    );
    d.f.dispatcher.clear();
    d.click("detail-node-shell");
    assert_eq!(
        sent_node_shell(&d.f),
        [Command::NodeShell {
            target: node_ref("worker-1")
        }]
    );
}

#[gpui::test]
fn the_header_button_is_disabled_on_a_read_only_cluster(cx: &mut TestAppContext) {
    let mut d = Detail::with_exec(cx, [node().name("worker-1").build()]);
    d.open(&node_ref("worker-1"));
    d.f.sessions.set_read_only(&cluster(), true).unwrap();
    d.settle();
    d.f.dispatcher.clear();
    d.click("detail-node-shell");
    assert!(
        sent_node_shell(&d.f).is_empty(),
        "a disabled button sends nothing"
    );
}
