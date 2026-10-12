//! The node table: the story's columns, the live Pods column, and the cordon badge after a patch.

use std::time::Duration;

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::ColumnId;
use oxikube_app::command_bus::{CommandBus, CommandRegistry, DispatchContext, Outcome};
use oxikube_app::{Confirmation, MutationGuard};
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_ports::{Delta, DeltaBatch, PatchKind};
use oxikube_testkit::{ResourceCall, Timeline, node};
use oxikube_ui::table::TableDelegate as _;
use serde_json::json;

use super::{node_ref, pod_on, worker};
use crate::actions::tests::nodes_kind;
use crate::table::tests::fixture::Fixture;

fn open_nodes(
    cx: &mut TestAppContext,
    objects: impl IntoIterator<Item = oxikube_domain::Resource>,
) -> (Fixture, gpui::Entity<crate::table::ResourceTable>) {
    let mut f = Fixture::new(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    let ports = f.ports();
    ports.discovery.set_kinds([nodes_kind()]);
    for object in objects {
        ports.resources.insert(object);
    }
    f.connect_with([]);
    let table = f.open(nodes_kind());
    (f, table)
}

fn cell(
    f: &mut Fixture,
    table: &gpui::Entity<crate::table::ResourceTable>,
    row: usize,
    column: &str,
) -> String {
    f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let ix = d
                .layout()
                .visible_index(&ColumnId::new(column))
                .unwrap_or_else(|| panic!("{column} is shown"));
            d.cell_text(row, ix, cx)
        })
    })
}

#[gpui::test]
fn the_table_shows_the_stories_columns(cx: &mut TestAppContext) {
    let (mut f, table) = open_nodes(
        cx,
        [
            node()
                .name("cp-1")
                .role("control-plane")
                .taint("node-role.kubernetes.io/control-plane", "", "NoSchedule")
                .build(),
            node().name("cp-2").cordoned().build(),
        ],
    );
    let visible = f
        .vcx
        .update(|_, cx| table.read(cx).read_rows(cx, |d| d.layout().visible_ids()));
    let ids: Vec<&str> = visible.iter().map(ColumnId::as_str).collect();
    assert_eq!(
        ids,
        [
            "name",
            "status",
            "roles",
            "version",
            "os-image",
            "kernel",
            "internal-ip",
            "pods",
            "cpu",
            "cpu-percent",
            "memory",
            "memory-percent",
            "taints",
            "schedulable",
            "age"
        ]
    );
    assert_eq!(f.names(&table), ["cp-1", "cp-2"]);
    assert_eq!(cell(&mut f, &table, 0, "roles"), "control-plane");
    assert_eq!(cell(&mut f, &table, 0, "taints"), "1");
    assert_eq!(cell(&mut f, &table, 0, "schedulable"), "Yes");
    assert_eq!(
        cell(&mut f, &table, 1, "status"),
        "Ready,SchedulingDisabled"
    );
    assert_eq!(cell(&mut f, &table, 1, "schedulable"), "No");
    // No metrics source yet: the slots show a dash, not zero.
    assert_eq!(cell(&mut f, &table, 0, "cpu"), "–");
    assert_eq!(cell(&mut f, &table, 0, "cpu-percent"), "–");
    assert_eq!(cell(&mut f, &table, 0, "memory-percent"), "–");
}

#[gpui::test]
fn the_pods_column_counts_the_pods_on_each_node(cx: &mut TestAppContext) {
    let (mut f, table) = open_nodes(
        cx,
        [
            worker(),
            node().name("worker-2").build(),
            pod_on("a", "worker-1", ("1", "1Gi"), ("1", "1Gi")),
            pod_on("b", "worker-1", ("1", "1Gi"), ("1", "1Gi")),
            pod_on("c", "worker-2", ("1", "1Gi"), ("1", "1Gi")),
        ],
    );
    // The Pod feed the table leased lists a moment after the nodes.
    f.settle();
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(f.names(&table), ["worker-1", "worker-2"]);
    assert_eq!(cell(&mut f, &table, 0, "pods"), "2");
    assert_eq!(cell(&mut f, &table, 1, "pods"), "1");
}

#[gpui::test]
fn the_cordon_badge_appears_once_the_patch_lands(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    let ports = f.ports();
    ports.discovery.set_kinds([nodes_kind()]);
    // The API server's watch: the node as listed, then (two seconds later, after our patch) the
    // same node with `spec.unschedulable`.
    let plain = node().name("worker-1").build();
    let after = node().name("worker-1").cordoned().build();
    ports.resources.script().watch.push_ok(
        Timeline::immediate([DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
            plain.clone(),
        ])])])
        .ok_at(
            Duration::from_secs(2),
            DeltaBatch::from_deltas(vec![Delta::Applied(after)]),
        )
        .keep_open(),
    );
    ports.resources.insert(plain);
    f.connect_with([]);
    let table = f.open(nodes_kind());
    assert_eq!(cell(&mut f, &table, 0, "status"), "Ready");
    assert_eq!(cell(&mut f, &table, 0, "schedulable"), "Yes");

    // Cordon it through the real guard and handler over the fake port.
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::nodes", oxikube_app::nodes::register_commands)
        .unwrap();
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(f.sessions.clone(), f.state.clone(), f.clock.clone()),
    );
    let command = Command::NodeCordon {
        target: node_ref("worker-1"),
    };
    let ctx = DispatchContext::new(Initiator::Ui, "alice");
    let Ok(Outcome::NeedsConfirmation(request)) =
        block_on(bus.dispatch(command.clone(), ctx.clone()))
    else {
        panic!("a confirmation is asked first");
    };
    block_on(bus.dispatch(
        command,
        ctx.with_confirmation(Confirmation::simple(request.token)),
    ))
    .expect("the cordon runs");
    let patches: Vec<_> = ports
        .resources
        .mutating_calls()
        .into_iter()
        .filter_map(|call| match call {
            ResourceCall::Patch { patch, options, .. } => {
                assert_eq!(patch.kind, PatchKind::Merge);
                Some((options.dry_run, patch.body))
            }
            _ => None,
        })
        .collect();
    let body = json!({"spec": {"unschedulable": true}});
    assert_eq!(patches, [(true, body.clone()), (false, body)]);

    // The watch tells the table.
    ports.resources.clock().advance(Duration::from_secs(2));
    f.settle();
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(
        cell(&mut f, &table, 0, "status"),
        "Ready,SchedulingDisabled"
    );
    assert_eq!(cell(&mut f, &table, 0, "schedulable"), "No");
}

#[gpui::test]
fn the_pods_column_follows_the_pods_without_the_node_changing(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    let ports = f.ports();
    ports.discovery.set_kinds([nodes_kind()]);
    let first = pod_on("a", "worker-1", ("1", "1Gi"), ("1", "1Gi"));
    let second = pod_on("b", "worker-1", ("1", "1Gi"), ("1", "1Gi"));
    // The Node feed first, then the Pod feed the table leases: one pod, a second two seconds on,
    // the first gone four seconds on.
    ports.resources.script().watch.push_ok(
        Timeline::immediate([DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
            worker(),
        ])])])
        .keep_open(),
    );
    ports.resources.script().watch.push_ok(
        Timeline::immediate([DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
            first.clone(),
        ])])])
        .ok_at(
            Duration::from_secs(2),
            DeltaBatch::from_deltas(vec![Delta::Applied(second)]),
        )
        .ok_at(
            Duration::from_secs(4),
            DeltaBatch::from_deltas(vec![Delta::Deleted(first)]),
        )
        .keep_open(),
    );
    f.connect_with([]);
    let table = f.open(nodes_kind());
    f.settle();
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(cell(&mut f, &table, 0, "pods"), "1");

    for (feed_secs, want) in [(2, "2"), (2, "1")] {
        ports
            .resources
            .clock()
            .advance(Duration::from_secs(feed_secs));
        f.settle();
        // The table's once-a-second tick notices the changed cell and redraws.
        f.vcx.executor().advance_clock(Duration::from_millis(1100));
        f.settle();
        f.vcx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(cell(&mut f, &table, 0, "pods"), want);
    }
}
