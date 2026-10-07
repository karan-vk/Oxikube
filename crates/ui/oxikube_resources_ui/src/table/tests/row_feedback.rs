//! Row keys that cannot run say why, and Shell, Attach and Debug act on the cursor row of a
//! selection and say so (E07-U563).

use gpui::TestAppContext;
use oxikube_domain::Resource;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::Gvk;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_testkit::deployment;

use super::fixture::{Fixture, cluster};
use super::p;
use crate::actions::tests::{nodes_kind, readonly_kind, toasts};

fn deployments_kind() -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new("apps", "v1", "Deployment"),
        preferred: true,
        plural: "deployments".into(),
        singular: "deployment".into(),
        short_names: vec!["deploy".into()],
        categories: vec!["all".into()],
        verbs: VerbSet::from_names(["get", "list", "watch", "delete"]),
        namespaced: true,
    }
}

fn sent_exec(f: &Fixture) -> Vec<Command> {
    f.dispatcher
        .sent()
        .into_iter()
        .filter(Command::is_exec)
        .collect()
}

fn at_version_1(mut resource: Resource) -> Resource {
    resource.meta.resource_version = Some("1".into());
    resource
}

fn messages(f: &mut Fixture) -> Vec<String> {
    toasts(f)
        .into_iter()
        .map(|toast| toast.message.to_string())
        .collect()
}

#[gpui::test]
fn shell_attach_and_debug_on_a_deployment_table_say_which_kinds_have_them(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    f.ports().discovery.set_kinds([deployments_kind()]);
    f.ports().resources.insert(at_version_1(
        deployment().namespace("x").name("web").build(),
    ));
    f.connect_with([]);
    let table = f.open(deployments_kind());
    f.keys(&table, "down");

    f.keys(&table, "s");
    assert_eq!(messages(&mut f), ["Shell is available for Pods and Nodes"]);
    f.keys(&table, "a");
    assert!(messages(&mut f).contains(&"Attach is available for Pods".to_owned()));
    f.keys(&table, "shift-d");
    assert!(messages(&mut f).contains(&"Debug is available for Pods".to_owned()));
    assert!(f.dispatcher.sent().is_empty(), "nothing was dispatched");
}

#[gpui::test]
fn attach_on_a_node_table_says_it_is_for_pods(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    f.ports().discovery.set_kinds([nodes_kind()]);
    f.ports().resources.insert(at_version_1(
        oxikube_testkit::node().name("worker-1").build(),
    ));
    f.connect_with([]);
    let table = f.open(nodes_kind());
    f.keys(&table, "down");
    f.keys(&table, "a");
    assert_eq!(messages(&mut f), ["Attach is available for Pods"]);
}

#[gpui::test]
fn delete_on_a_kind_the_server_cannot_delete_names_the_kind(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    f.connect_with([]);
    let table = f.open(readonly_kind());
    let target = ResourceRef::new(
        cluster(),
        Gvk::new("", "v1", "ComponentStatus"),
        None,
        "etcd-0",
    );
    f.vcx.update(|window, cx| {
        table.update(cx, |table, cx| {
            table.run_action(CommandId::RESOURCE_DELETE, vec![target], window, cx)
        })
    });
    f.settle();
    assert_eq!(
        messages(&mut f),
        ["Delete is not available for ComponentStatus"]
    );
}

#[gpui::test]
fn a_key_with_no_row_to_act_on_says_so(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    f.connect_with([]);
    let table = f.open_pods();
    f.keys(&table, "s");
    assert_eq!(messages(&mut f), ["Select a row first"]);
}

#[gpui::test]
fn shell_and_attach_on_several_selected_pods_act_on_the_cursor_row_and_say_so(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::with_exec(cx);
    f.connect_with([
        p("x", "web-0", "1"),
        p("x", "web-1", "1"),
        p("x", "web-2", "1"),
    ]);
    let table = f.open_pods();
    f.update(&table, |t, cx| {
        t.move_cursor(1, false, cx); // web-0
        t.move_cursor(1, true, cx); // extend to web-1
        t.move_cursor(1, true, cx); // extend to web-2: the cursor
    });
    assert_eq!(f.selected(&table).len(), 3);
    f.dispatcher.clear();

    f.keys(&table, "s");
    let sent = sent_exec(&f);
    assert!(
        matches!(sent.as_slice(), [Command::PodShell { target, .. }] if &*target.name == "web-2"),
        "{sent:?}"
    );
    assert_eq!(
        messages(&mut f),
        ["Shell acts on the cursor row (web-2), not the 3 selected"]
    );

    f.keys(&table, "a");
    let sent = sent_exec(&f);
    assert!(
        matches!(sent.last(), Some(Command::PodAttach { target, .. }) if &*target.name == "web-2"),
        "{sent:?}"
    );
    assert!(
        messages(&mut f)
            .contains(&"Attach acts on the cursor row (web-2), not the 3 selected".to_owned())
    );
}

#[gpui::test]
fn debug_on_several_selected_pods_opens_the_dialog_for_the_cursor_row(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    f.connect_with([p("x", "web-0", "1"), p("x", "web-1", "1")]);
    let table = f.open_pods();
    f.update(&table, |t, cx| {
        t.move_cursor(1, false, cx);
        t.move_cursor(1, true, cx);
    });
    assert_eq!(f.selected(&table).len(), 2);
    f.keys(&table, "shift-d");
    assert_eq!(
        messages(&mut f),
        ["Debug acts on the cursor row (web-1), not the 2 selected"]
    );
}

#[gpui::test]
fn one_selected_row_acts_without_a_note(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    f.connect_with([p("x", "web-0", "1"), p("x", "web-1", "1")]);
    let table = f.open_pods();
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.keys(&table, "s");
    assert!(messages(&mut f).is_empty(), "{:?}", messages(&mut f));
}
