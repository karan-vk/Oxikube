//! The pod keys (E12-S03), pressed in a real Pod table with the shipped keymap: `ctrl-k` force
//! deletes the cursor pod, `z` sweeps the Completed and Failed pods in view, `o` shows the node and
//! `shift-j` jumps to the owner. Each dispatches the command the palette, the context menu and an
//! agent run; the guard's read-only block applies to the two that change the cluster.

use gpui::{Entity, TestAppContext};
use oxikube_domain::Resource;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_testkit::pod;

use super::fixture::{Fixture, cluster};
use super::p;
use crate::actions::tests::toasts;
use crate::table::ResourceTable;

fn target(name: &str) -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "x", name)
}

fn phased(
    name: &str,
    build: impl FnOnce(oxikube_testkit::builders::PodBuilder) -> oxikube_testkit::builders::PodBuilder,
) -> Resource {
    let mut r = build(pod().namespace("x").name(name)).build();
    r.meta.resource_version = Some("1".into());
    r
}

fn open(cx: &mut TestAppContext, pods: Vec<Resource>) -> (Fixture, Entity<ResourceTable>) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with(pods);
    let table = f.open_pods();
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.dispatcher.clear();
    (f, table)
}

fn messages(f: &mut Fixture) -> Vec<String> {
    toasts(f)
        .into_iter()
        .map(|t| t.message.to_string())
        .collect()
}

#[gpui::test]
fn ctrl_k_force_deletes_the_cursor_pod(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx, vec![p("x", "web-0", "1"), p("x", "web-1", "1")]);
    f.keys(&table, "down ctrl-k");
    assert_eq!(
        f.dispatcher.sent(),
        [Command::PodForceDelete {
            target: target("web-1")
        }]
    );
}

#[gpui::test]
fn z_sweeps_the_completed_and_failed_pods_of_the_view(cx: &mut TestAppContext) {
    let (mut f, table) = open(
        cx,
        vec![
            phased("a-done", |b| b.succeeded()),
            phased("b-running", |b| b.running()),
            phased("c-failed", |b| b.failed()),
            phased("d-pending", |b| b.pending()),
        ],
    );
    f.keys(&table, "z");
    assert_eq!(
        f.dispatcher.sent(),
        [Command::PodSanitize {
            cluster: cluster(),
            namespace: Some("x".into()),
            targets: vec![target("a-done"), target("c-failed")],
        }]
    );
}

#[gpui::test]
fn z_sweeps_only_what_the_filter_leaves_in_view(cx: &mut TestAppContext) {
    let (mut f, table) = open(
        cx,
        vec![
            phased("job-a", |b| b.succeeded()),
            phased("job-b", |b| b.failed()),
            phased("web-done", |b| b.succeeded()),
        ],
    );
    let dispatcher = f.deps.dispatcher.clone();
    f.vcx.update(|_, cx| {
        dispatcher.dispatch(
            Command::TableSetFilter {
                cluster: cluster(),
                gvk: Gvk::new("", "v1", "Pod"),
                text: "^job".into(),
            },
            cx,
        )
    });
    f.settle();
    assert_eq!(f.names(&table), ["job-a", "job-b"]);
    f.dispatcher.clear();
    f.keys(&table, "z");
    let sent = f.dispatcher.sent();
    let [Command::PodSanitize { targets, .. }] = sent.as_slice() else {
        panic!("one sweep: {sent:?}");
    };
    assert_eq!(targets, &[target("job-a"), target("job-b")]);
}

#[gpui::test]
fn z_says_so_when_nothing_in_view_has_finished(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx, vec![phased("web-0", |b| b.running())]);
    f.keys(&table, "z");
    assert!(f.dispatcher.sent().is_empty());
    assert!(
        messages(&mut f)
            .iter()
            .any(|m| m.contains("No Completed or Failed pods")),
        "{:?}",
        messages(&mut f)
    );
}

#[gpui::test]
fn z_sweeps_the_view_even_with_rows_selected(cx: &mut TestAppContext) {
    let (mut f, table) = open(
        cx,
        vec![
            phased("a-done", |b| b.succeeded()),
            phased("b-running", |b| b.running()),
        ],
    );
    f.keys(&table, "shift-down");
    assert_eq!(f.selected(&table).len(), 2);
    f.keys(&table, "z");
    let sent = f.dispatcher.sent();
    assert!(
        matches!(sent.as_slice(), [Command::PodSanitize { targets, .. }] if targets.len() == 1),
        "{sent:?}"
    );
}

#[gpui::test]
fn o_and_shift_j_open_the_node_and_the_owner(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx, vec![p("x", "web-0", "1")]);
    f.keys(&table, "o shift-j");
    assert_eq!(
        f.dispatcher.sent(),
        [
            Command::PodShowNode {
                target: target("web-0")
            },
            Command::PodJumpToOwner {
                target: target("web-0")
            },
        ]
    );
}

#[gpui::test]
fn shift_down_still_extends_the_selection_on_a_pod_table(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx, vec![p("x", "web-0", "1"), p("x", "web-1", "1")]);
    f.keys(&table, "shift-down");
    assert_eq!(f.selected(&table), ["web-0", "web-1"]);
    assert!(f.dispatcher.sent().is_empty());
}

#[gpui::test]
fn the_keys_are_text_while_the_filter_field_has_the_focus(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx, vec![phased("web-0", |b| b.succeeded())]);
    f.keys(&table, "/ z o");
    let verbs: Vec<_> = f
        .dispatcher
        .sent()
        .into_iter()
        .filter(|c| !matches!(c, Command::TableFocusFilter { .. }))
        .collect();
    assert_eq!(verbs, [], "typed into the filter, not run");
}

#[gpui::test]
fn a_read_only_cluster_greys_out_the_mutating_pod_actions_and_the_keys_say_why(
    cx: &mut TestAppContext,
) {
    let (mut f, table) = open(cx, vec![phased("web-0", |b| b.succeeded())]);
    f.sessions.set_read_only(&cluster(), true).unwrap();
    let entries = f.vcx.update(|_, cx| table.read(cx).action_entries(cx));
    for id in [
        CommandId::POD_EVICT,
        CommandId::POD_DELETE,
        CommandId::POD_FORCE_DELETE,
        CommandId::POD_SANITIZE,
    ] {
        let entry = entries
            .iter()
            .find(|e| e.command() == id)
            .unwrap_or_else(|| panic!("{id} is listed"));
        assert!(!entry.is_enabled(), "{id}");
        assert_eq!(entry.reason().as_deref(), Some("This cluster is read-only"));
    }
    // Navigation changes nothing, so it stays available.
    for id in [CommandId::POD_JUMP_TO_OWNER, CommandId::POD_SHOW_NODE] {
        let entry = entries.iter().find(|e| e.command() == id).unwrap();
        assert!(entry.is_enabled(), "{id}");
    }
    for key in ["ctrl-k", "z"] {
        f.keys(&table, key);
    }
    assert!(f.dispatcher.sent().is_empty(), "{:?}", f.dispatcher.sent());
    assert!(
        messages(&mut f).iter().any(|m| m.contains("read-only")),
        "{:?}",
        messages(&mut f)
    );
}

#[gpui::test]
fn the_palette_lists_the_pod_commands_for_a_pod_table(cx: &mut TestAppContext) {
    use oxikube_workspace::command_surface::CommandSurface as _;
    let (mut f, table) = open(cx, vec![p("x", "web-0", "1")]);
    let own = f.vcx.update(|_, cx| table.read(cx).own_commands(cx));
    for id in [
        CommandId::POD_EVICT,
        CommandId::POD_DELETE,
        CommandId::POD_FORCE_DELETE,
        CommandId::POD_SANITIZE,
        CommandId::POD_JUMP_TO_OWNER,
        CommandId::POD_SHOW_NODE,
        CommandId::POD_VIEW_LOGS,
    ] {
        assert!(own.contains(&id), "{id} in {own:?}");
    }
    // `resource::Delete` is the same dialog for a pod, so the palette hands it to the table too.
    assert!(own.contains(&CommandId::RESOURCE_DELETE));
}
