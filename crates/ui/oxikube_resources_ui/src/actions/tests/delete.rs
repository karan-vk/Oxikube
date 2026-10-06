//! The delete dialog: confirm tiers, propagation, the key, and what reaches the cluster.

use gpui::TestAppContext;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::Propagation;
use oxikube_domain::safety::ConfirmTier;
use oxikube_ports::PropagationPolicy;
use oxikube_testkit::{ResourceCall, node};

use super::{dialog, nodes_kind, toasts, with_dialog};
use crate::actions::Stage;
use crate::table::tests::fixture::Fixture;
use crate::table::tests::p;

fn deletes(f: &Fixture) -> Vec<(bool, Option<PropagationPolicy>, String)> {
    f.ports()
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

#[gpui::test]
fn deleting_a_pod_is_one_confirm_and_one_guarded_command(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with([p("x", "web-0", "1"), p("x", "web-1", "1")]);
    let table = f.open_pods();
    f.keys(&table, "down"); // the cursor row: web-0
    f.keys(&table, "delete");
    let dialog = dialog(&mut f).expect("the delete key opens the dialog");
    assert!(deletes(&f).is_empty());
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(f.vcx.debug_bounds("delete-dialog").is_some());
    assert!(f.vcx.debug_bounds("delete-what").is_some());
    assert!(
        f.vcx.debug_bounds("delete-type-input").is_none(),
        "no typing for a Pod"
    );
    assert!(f.vcx.debug_bounds("delete-warning").is_none());

    with_dialog(&mut f, &dialog, |d, _, cx| {
        assert_eq!(d.stage(), Stage::Confirm);
        assert!(d.can_confirm(cx), "an ordinary object needs no typing");
        assert!(d.plan().phrase().is_none());
        d.confirm(cx);
    });
    f.settle();

    // The server rehearses the delete, then it runs, with the kubectl default propagation.
    assert_eq!(
        deletes(&f),
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
        ]
    );
    // Through the guard: one audit record, from the UI.
    let audit = f.state.audit_log();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(audit[0].initiator, Initiator::Ui);
    assert_eq!(&*audit[0].cmd, "resource::Delete");
    assert_eq!(&*audit[0].who, "alice");
    // One object that went well closes the dialog with a toast.
    assert!(super::dialog(&mut f).is_none());
    assert!(
        toasts(&mut f)
            .iter()
            .any(|t| t.message.as_ref() == "Deleted 1 object"),
        "{:?}",
        toasts(&mut f)
    );
}

#[gpui::test]
fn ctrl_d_opens_the_same_dialog(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with([p("x", "web-0", "1")]);
    let table = f.open_pods();
    f.keys(&table, "down");
    f.keys(&table, "ctrl-d");
    assert!(dialog(&mut f).is_some());
    assert!(deletes(&f).is_empty(), "the key never deletes by itself");
}

#[gpui::test]
fn cancel_sends_nothing(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with([p("x", "web-0", "1")]);
    let table = f.open_pods();
    f.keys(&table, "down");
    f.keys(&table, "delete");
    let d = dialog(&mut f).unwrap();
    with_dialog(&mut f, &d, |d, _, cx| d.cancel(cx));
    assert!(dialog(&mut f).is_none());
    assert!(deletes(&f).is_empty());
    assert!(f.state.audit_log().is_empty());
}

#[gpui::test]
fn a_node_needs_its_name_typed_and_a_wrong_name_deletes_nothing(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    let ports = f.ports();
    ports.discovery.set_kinds([nodes_kind()]);
    ports.resources.insert(node().name("worker-1").build());
    f.connect_with([]);
    let table = f.open(nodes_kind());
    assert_eq!(f.names(&table), ["worker-1"]);
    f.keys(&table, "down");
    f.keys(&table, "delete");
    let d = dialog(&mut f).expect("dialog");
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(f.vcx.debug_bounds("delete-type-prompt").is_some());
    assert!(f.vcx.debug_bounds("delete-type-input").is_some());
    assert!(
        f.vcx.debug_bounds("delete-warning").is_some(),
        "a Node says what is at stake"
    );

    with_dialog(&mut f, &d, |d, window, cx| {
        assert_eq!(d.plan().tier(), ConfirmTier::TypeName);
        assert_eq!(d.plan().phrase(), Some("worker-1"));
        assert!(!d.can_confirm(cx), "nothing typed yet");
        d.type_text("worker-2", window, cx);
    });
    with_dialog(&mut f, &d, |d, _, cx| {
        assert!(!d.can_confirm(cx), "a wrong name keeps Delete off");
        d.confirm(cx);
    });
    assert!(
        deletes(&f).is_empty(),
        "confirming with a wrong name sends nothing"
    );
    assert!(f.state.audit_log().is_empty());
    assert_eq!(d.read_with(&f.vcx, |d, _| d.stage()), Stage::Confirm);

    with_dialog(&mut f, &d, |d, window, cx| {
        d.type_text("worker-1", window, cx)
    });
    with_dialog(&mut f, &d, |d, _, cx| {
        assert!(d.can_confirm(cx));
        d.confirm(cx);
    });
    f.settle();
    assert_eq!(deletes(&f).len(), 2);
    assert_eq!(f.state.audit_log().len(), 1);
}

#[gpui::test]
fn the_propagation_choice_raises_the_confirmation_and_reaches_the_request(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with([p("x", "web-0", "1")]);
    let table = f.open_pods();
    f.keys(&table, "down");
    f.keys(&table, "delete");
    let d = dialog(&mut f).unwrap();
    with_dialog(&mut f, &d, |d, window, cx| {
        assert_eq!(d.plan().propagation(), Propagation::Background);
        d.set_propagation(Propagation::Foreground, cx);
        // A cascading delete takes the typed name, like the guard will ask.
        assert_eq!(d.plan().tier(), ConfirmTier::TypeName);
        assert!(!d.can_confirm(cx));
        d.type_text("web-0", window, cx);
    });
    with_dialog(&mut f, &d, |d, _, cx| {
        assert!(d.can_confirm(cx));
        d.confirm(cx);
    });
    f.settle();
    let sent = deletes(&f);
    assert_eq!(sent.len(), 2);
    assert!(
        sent.iter()
            .all(|(_, policy, _)| *policy == Some(PropagationPolicy::Foreground))
    );
}

#[gpui::test]
fn orphaning_dependents_is_still_a_plain_confirm(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with([p("x", "web-0", "1")]);
    let table = f.open_pods();
    f.keys(&table, "down");
    f.keys(&table, "delete");
    let d = dialog(&mut f).unwrap();
    with_dialog(&mut f, &d, |d, _, cx| {
        d.set_propagation(Propagation::Orphan, cx);
        assert_eq!(d.plan().tier(), ConfirmTier::Simple);
        assert!(d.can_confirm(cx));
        d.confirm(cx);
    });
    f.settle();
    let sent = deletes(&f);
    assert!(
        sent.iter()
            .all(|(_, policy, _)| *policy == Some(PropagationPolicy::Orphan))
    );
}
