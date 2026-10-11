//! Cordon and uncordon in the row menu and on `c` / `u`.

use gpui::TestAppContext;
use oxikube_domain::command::{Command, CommandId};
use oxikube_ports::{ClusterPrefs, ClusterPrefsTable};
use oxikube_testkit::node;

use super::node_ref;
use crate::actions::tests::nodes_kind;
use crate::table::tests::fixture::{Fixture, cluster};

fn with_node(cx: &mut TestAppContext) -> (Fixture, gpui::Entity<crate::table::ResourceTable>) {
    let mut f = Fixture::with_exec(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    let ports = f.ports();
    ports.discovery.set_kinds([nodes_kind()]);
    ports.resources.insert(node().name("worker-1").build());
    f.connect_with([]);
    let table = f.open(nodes_kind());
    (f, table)
}

fn sent(f: &Fixture) -> Vec<Command> {
    f.dispatcher
        .sent()
        .into_iter()
        .filter(|c| matches!(c, Command::NodeCordon { .. } | Command::NodeUncordon { .. }))
        .collect()
}

#[gpui::test]
fn c_cordons_and_u_uncordons_the_cursor_row(cx: &mut TestAppContext) {
    let (mut f, table) = with_node(cx);
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.dispatcher.clear();
    f.keys(&table, "c");
    assert_eq!(
        sent(&f),
        [Command::NodeCordon {
            target: node_ref("worker-1")
        }]
    );
    f.dispatcher.clear();
    f.keys(&table, "u");
    assert_eq!(
        sent(&f),
        [Command::NodeUncordon {
            target: node_ref("worker-1")
        }]
    );
}

#[gpui::test]
fn the_menu_offers_both_for_a_node_and_neither_for_a_pod(cx: &mut TestAppContext) {
    let (mut f, table) = with_node(cx);
    let commands: Vec<CommandId> = f.vcx.update(|_, cx| {
        table
            .read(cx)
            .action_entries(cx)
            .iter()
            .map(|e| e.command())
            .collect()
    });
    assert!(commands.contains(&CommandId::NODE_CORDON), "{commands:?}");
    assert!(commands.contains(&CommandId::NODE_UNCORDON), "{commands:?}");

    f.connect_pods_for_menu();
}

impl Fixture {
    /// A pods table in the same window offers neither.
    fn connect_pods_for_menu(&mut self) {
        let table = self.open_pods();
        let commands: Vec<CommandId> = self.vcx.update(|_, cx| {
            table
                .read(cx)
                .action_entries(cx)
                .iter()
                .map(|e| e.command())
                .collect()
        });
        assert!(!commands.contains(&CommandId::NODE_CORDON), "{commands:?}");
        assert!(
            !commands.contains(&CommandId::NODE_UNCORDON),
            "{commands:?}"
        );
    }
}

#[gpui::test]
fn a_read_only_cluster_greys_both_out_with_the_reason(cx: &mut TestAppContext) {
    let (mut f, table) = with_node(cx);
    f.sessions.set_prefs_table(
        ClusterPrefsTable::new(ClusterPrefs::default()).with_cluster(
            cluster(),
            ClusterPrefs {
                read_only: true,
                ..ClusterPrefs::default()
            },
        ),
    );
    f.settle();
    let entries = f.vcx.update(|_, cx| table.read(cx).action_entries(cx));
    for command in [CommandId::NODE_CORDON, CommandId::NODE_UNCORDON] {
        let entry = entries
            .iter()
            .find(|e| e.command() == command)
            .expect("offered");
        assert!(!entry.is_enabled(), "{command} mutates");
        assert!(entry.reason().expect("a reason").contains("read-only"));
    }
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.dispatcher.clear();
    f.keys(&table, "c");
    assert!(sent(&f).is_empty(), "nothing is sent");
}

#[gpui::test]
fn c_and_u_do_nothing_on_a_pods_table(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    f.connect_with([crate::table::tests::p("x", "web-0", "1")]);
    let table = f.open_pods();
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.dispatcher.clear();
    f.keys(&table, "c");
    f.keys(&table, "u");
    assert!(sent(&f).is_empty(), "{:?}", f.dispatcher.sent());
}
