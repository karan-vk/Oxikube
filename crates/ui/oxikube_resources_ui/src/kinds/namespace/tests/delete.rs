//! Deleting a namespace: the type-the-name dialog with its warnings, the counts, the audit, and
//! what a read-only cluster does.

use gpui::TestAppContext;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_domain::view::NoteLevel;
use oxikube_testkit::ResourceCall;
use serde_json::json;

use super::{Env, ns};
use crate::actions::Stage;
use crate::actions::tests::{dialog, toasts, with_dialog};
use crate::table::tests::fixture::{Fixture, cluster};

fn deletes(f: &Fixture) -> Vec<(bool, String)> {
    f.ports()
        .resources
        .mutating_calls()
        .into_iter()
        .map(|call| match call {
            ResourceCall::Delete {
                options,
                name,
                kind,
                ..
            } => {
                assert_eq!(&*kind.kind, "Namespace");
                (options.dry_run, name)
            }
            other => panic!("unexpected write {other:?}"),
        })
        .collect()
}

#[gpui::test]
fn delete_asks_for_the_name_and_says_what_goes_with_the_namespace(cx: &mut TestAppContext) {
    let mut env = Env::standard(cx);
    env.cursor_to("team-a");
    env.keys("ctrl-d");
    let d = dialog(&mut env.f).expect("ctrl-d opens the delete dialog");
    assert!(deletes(&env.f).is_empty());

    env.f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(env.f.vcx.debug_bounds("delete-type-prompt").is_some());
    assert!(env.f.vcx.debug_bounds("delete-type-input").is_some());
    assert!(
        env.f.vcx.debug_bounds("delete-note-0").is_some(),
        "the counts are on screen"
    );
    with_dialog(&mut env.f, &d, |d, _, cx| {
        let plan = d.plan();
        assert_eq!(plan.tier(), ConfirmTier::TypeName);
        assert_eq!(plan.risk(), Risk::Irreversible);
        assert_eq!(plan.phrase(), Some("team-a"));
        assert_eq!(plan.items()[0].command.id().as_str(), "namespace::Delete");
        let notes: Vec<&str> = d.notes().iter().map(|n| n.text.as_str()).collect();
        assert!(
            notes.iter().any(|n| n.contains("2 Pods")),
            "team-a holds two pods: {notes:?}"
        );
        assert!(!d.can_confirm(cx));
    });
}

#[gpui::test]
fn the_wrong_name_deletes_nothing_the_right_one_deletes_and_audits_the_counts(
    cx: &mut TestAppContext,
) {
    let mut env = Env::standard(cx);
    env.cursor_to("team-a");
    env.keys("delete");
    let d = dialog(&mut env.f).expect("dialog");

    with_dialog(&mut env.f, &d, |d, window, cx| {
        d.type_text("team", window, cx)
    });
    with_dialog(&mut env.f, &d, |d, _, cx| {
        assert!(!d.can_confirm(cx), "a partial name keeps Delete off");
        d.confirm(cx);
    });
    assert!(deletes(&env.f).is_empty());
    assert!(env.f.state.audit_log().is_empty());
    assert_eq!(d.read_with(&env.f.vcx, |d, _| d.stage()), Stage::Confirm);

    with_dialog(&mut env.f, &d, |d, window, cx| {
        d.type_text("team-a", window, cx)
    });
    with_dialog(&mut env.f, &d, |d, _, cx| {
        assert!(d.can_confirm(cx));
        d.confirm(cx);
    });
    env.f.settle();

    // The server rehearses the delete, then it runs.
    assert_eq!(
        deletes(&env.f),
        [(true, "team-a".to_owned()), (false, "team-a".to_owned())]
    );
    let audit = env.f.state.audit_log();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(audit[0].initiator, Initiator::Ui);
    assert_eq!(&*audit[0].cmd, "namespace::Delete");
    assert_eq!(&*audit[0].target.name, "team-a");
    assert_eq!(
        audit[0].detail.as_deref(),
        Some("contents=Deployment:0,Pod:2"),
        "the record says what was in it (deployments are watched too, and there are none)"
    );
    // One object that went well closes the dialog with a toast.
    assert!(dialog(&mut env.f).is_none());
    assert!(
        toasts(&mut env.f)
            .iter()
            .any(|t| t.message.as_ref() == "Deleted 1 object"),
        "{:?}",
        toasts(&mut env.f)
    );
}

#[gpui::test]
fn a_system_namespace_gets_an_extra_warning_and_the_same_typed_name(cx: &mut TestAppContext) {
    let mut env = Env::standard(cx);
    env.cursor_to("kube-system");
    env.keys("ctrl-d");
    let d = dialog(&mut env.f).expect("dialog");
    with_dialog(&mut env.f, &d, |d, _, _| {
        assert_eq!(d.plan().tier(), ConfirmTier::TypeName);
        assert_eq!(d.plan().phrase(), Some("kube-system"));
        let first = &d.notes()[0];
        assert_eq!(first.level, NoteLevel::Danger, "the worst note comes first");
        assert!(first.text.contains("breaks the cluster"), "{}", first.text);
    });
}

#[gpui::test]
fn hnc_children_and_subnamespaces_are_warned_about(cx: &mut TestAppContext) {
    let mut env = Env::new(
        cx,
        [
            ns(
                "team-a",
                json!({ "labels": { "team-a.tree.hnc.x-k8s.io/depth": "0" } }),
            ),
            ns(
                "team-a-dev",
                json!({
                    "annotations": { "hnc.x-k8s.io/subnamespace-of": "team-a" },
                    "labels": { "team-a.tree.hnc.x-k8s.io/depth": "1" },
                }),
            ),
        ],
        [],
    );
    env.cursor_to("team-a");
    env.keys("ctrl-d");
    let d = dialog(&mut env.f).expect("dialog");
    with_dialog(&mut env.f, &d, |d, _, _| {
        assert!(
            d.notes()
                .iter()
                .any(|n| n.text.contains("1 HNC subnamespace sits below")),
            "{:?}",
            d.notes()
        );
    });
    with_dialog(&mut env.f, &d, |d, _, cx| d.cancel(cx));

    env.cursor_to("team-a-dev");
    env.keys("ctrl-d");
    let d = dialog(&mut env.f).expect("dialog");
    with_dialog(&mut env.f, &d, |d, _, _| {
        assert!(
            d.notes()
                .iter()
                .any(|n| n.text.contains("HNC subnamespace of team-a")),
            "{:?}",
            d.notes()
        );
    });
}

#[gpui::test]
fn a_read_only_cluster_says_so_instead_of_opening_the_dialog(cx: &mut TestAppContext) {
    let mut env = Env::standard(cx);
    env.f.sessions.set_read_only(&cluster(), true).unwrap();
    env.cursor_to("team-a");
    env.keys("ctrl-d");
    assert!(dialog(&mut env.f).is_none());
    assert!(!toasts(&mut env.f).is_empty(), "a toast says why");
    assert!(deletes(&env.f).is_empty());
}

#[gpui::test]
fn deleting_several_namespaces_asks_for_the_cluster_name_and_names_whose_warning(
    cx: &mut TestAppContext,
) {
    let mut env = Env::standard(cx);
    env.cursor_to("kube-system");
    env.keys("shift-down shift-down");
    assert_eq!(env.f.selected(&env.table).len(), 3);
    env.keys("ctrl-d");
    let d = dialog(&mut env.f).expect("dialog");
    with_dialog(&mut env.f, &d, |d, _, _| {
        assert_eq!(d.plan().items().len(), 3);
        assert_eq!(
            d.plan().phrase(),
            Some("kind"),
            "the cluster's context name"
        );
        assert!(
            d.notes()
                .iter()
                .any(|n| n.text.starts_with("kube-system: ")),
            "with several, each warning says which namespace: {:?}",
            d.notes()
        );
    });
}
