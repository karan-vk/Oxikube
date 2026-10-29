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

    // The guard dry-runs the irreversible delete before it asks, the handler rehearses it once
    // more, then it runs.
    assert_eq!(
        deletes(&env.f),
        [
            (true, "team-a".to_owned()),
            (true, "team-a".to_owned()),
            (false, "team-a".to_owned())
        ]
    );
    let audit = env.f.state.audit_log();
    assert_eq!(
        audit.len(),
        2,
        "the guard's dry run, then the delete: {audit:?}"
    );
    assert!(audit[0].dry_run && !audit[1].dry_run);
    for record in &audit {
        assert_eq!(record.outcome, AuditOutcome::Succeeded);
        assert_eq!(record.initiator, Initiator::Ui);
        assert_eq!(&*record.cmd, "namespace::Delete");
        assert_eq!(&*record.target.name, "team-a");
        assert_eq!(
            record.detail.as_deref(),
            Some("contents=Deployment:0,Pod:2"),
            "the record says what was in it (deployments are watched too, and there are none)"
        );
    }
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

#[gpui::test]
fn many_notes_scroll_and_stop_at_a_cap_so_the_confirm_step_stays_on_screen(
    cx: &mut TestAppContext,
) {
    let names: Vec<String> = (0..60).map(|i| format!("team-{i:02}")).collect();
    let mut env = Env::new(
        cx,
        names.iter().map(|name| ns(name, json!({}))),
        names
            .iter()
            .map(|name| crate::table::tests::p(name, "web-0", "1")),
    );
    let table = env.table.clone();
    env.f.update(&table, |t, cx| t.select_all(cx));
    assert_eq!(env.f.selected(&env.table).len(), 60);
    env.keys("ctrl-d");
    let d = dialog(&mut env.f).expect("dialog");
    with_dialog(&mut env.f, &d, |d, _, _| {
        assert!(d.notes().len() > 40, "{} notes", d.notes().len());
    });
    env.f.settle();
    let bounds = |env: &mut Env, selector: &'static str| env.f.vcx.debug_bounds(selector);
    let notes = bounds(&mut env, "delete-notes").expect("the notes list draws");
    assert!(
        notes.size.height <= gpui::px(200.) + gpui::px(1.),
        "the list scrolls past its cap: {:?}",
        notes.size
    );
    assert!(bounds(&mut env, "delete-note-29").is_some());
    assert!(
        bounds(&mut env, "delete-note-30").is_none(),
        "rows past the cap are not built"
    );
    assert!(
        bounds(&mut env, "delete-notes-more").is_some(),
        "says how many more"
    );
    let window = env.f.vcx.update(|window, _| window.viewport_size());
    for selector in ["delete-type-input", "delete-confirm"] {
        let at = bounds(&mut env, selector).unwrap_or_else(|| panic!("{selector} draws"));
        assert!(
            at.bottom() <= window.height,
            "{selector} is on screen: {at:?} in {window:?}"
        );
    }
}
