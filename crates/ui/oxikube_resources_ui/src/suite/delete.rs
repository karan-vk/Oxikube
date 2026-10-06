//! Deleting from the table end to end: the key opens the dialog, confirming sends a guarded
//! `resource::Delete` through the bus (rehearsal, then the delete, one audit record), the feed
//! delivers the deletion and the row leaves; and a read-only cluster refuses all of it.

use gpui::TestAppContext;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_ports::PropagationPolicy;
use oxikube_testkit::{ResourceCall, ScriptedFeed};

use super::{Scripted, pod_in};
use crate::actions::tests::{dialog, toasts, with_dialog};
use crate::table::tests::fixture::{Fixture, cluster};

fn deletes(s: &Scripted) -> Vec<(bool, Option<PropagationPolicy>, String)> {
    s.f.ports()
        .resources
        .mutating_calls()
        .into_iter()
        .map(|call| match call {
            ResourceCall::Delete { options, name, .. } => {
                (options.dry_run, options.propagation, name)
            }
            other => panic!("unexpected write {other:?}"),
        })
        .collect()
}

fn open(cx: &mut TestAppContext) -> Scripted {
    let feed = ScriptedFeed::new()
        .initial([pod_in("x", "web-0", 0), pod_in("x", "web-1", 0)])
        // The server deletes web-0 after the request: its watch says so.
        .delete(1, pod_in("x", "web-0", 0));
    Scripted::open_in(Fixture::with_actions(cx), &feed)
}

#[gpui::test]
fn delete_goes_through_the_bus_the_guard_and_the_audit_and_the_row_leaves(cx: &mut TestAppContext) {
    let mut s = open(cx);
    s.f.keys(&s.table, "j delete");
    let dialog = dialog(&mut s.f).expect("the delete key opens the confirmation");
    assert!(
        deletes(&s).is_empty(),
        "nothing is sent before the user confirms"
    );
    with_dialog(&mut s.f, &dialog, |d, _, cx| {
        assert!(d.can_confirm(cx), "an ordinary object needs no typed name");
        d.confirm(cx);
    });
    s.f.settle();

    assert_eq!(
        deletes(&s),
        [
            (
                true,
                Some(PropagationPolicy::Background),
                "web-0".to_owned()
            ),
            (
                false,
                Some(PropagationPolicy::Background),
                "web-0".to_owned()
            ),
        ],
        "the server rehearses the delete, then it runs"
    );
    let audit = s.f.state.audit_log();
    assert_eq!(audit.len(), 1, "one command, one record");
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(audit[0].initiator, Initiator::Ui);
    assert_eq!(&*audit[0].cmd, "resource::Delete");

    // The watch reports the deletion and the table drops the row.
    assert_eq!(s.names(), ["web-0", "web-1"]);
    s.step();
    assert_eq!(s.names(), ["web-1"]);
}

#[gpui::test]
fn a_read_only_cluster_blocks_delete_before_anything_is_sent_and_lifting_it_unblocks(
    cx: &mut TestAppContext,
) {
    let mut s = open(cx);
    s.f.sessions
        .set_read_only(&cluster(), true)
        .expect("a known cluster");
    s.f.keys(&s.table, "j delete");
    assert!(
        dialog(&mut s.f).is_none(),
        "no dialog in a read-only cluster"
    );
    let toasts = toasts(&mut s.f);
    assert!(
        toasts.iter().any(|t| t.message.contains("read-only")),
        "the user is told why: {toasts:?}"
    );
    assert!(deletes(&s).is_empty(), "nothing reached the cluster");
    assert!(
        s.f.state.audit_log().is_empty(),
        "nothing reached the guard"
    );
    assert_eq!(s.names(), ["web-0", "web-1"]);

    // The palette's entry for the row is there, disabled, with the reason.
    let entries = s.f.vcx.update(|_, cx| s.table.read(cx).action_entries(cx));
    let delete = entries
        .iter()
        .find(|e| e.command() == oxikube_domain::command::CommandId::RESOURCE_DELETE)
        .expect("Delete is offered");
    assert!(!delete.is_enabled());
    assert!(
        delete.reason().is_some_and(|why| why.contains("read-only")),
        "{:?}",
        delete.reason()
    );

    // Lifting read-only unblocks the same key.
    s.f.sessions
        .set_read_only(&cluster(), false)
        .expect("a known cluster");
    s.f.keys(&s.table, "delete");
    assert!(dialog(&mut s.f).is_some(), "the dialog opens again");
}
