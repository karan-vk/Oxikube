//! The create-namespace dialog: reachable from the toolbar, validates as typed, creates through
//! `namespace::Create`.

use gpui::TestAppContext;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_testkit::ResourceCall;
use serde_json::json;

use super::{Env, ns};
use crate::actions::tests::toasts;
use crate::kinds::namespace::{CreateNamespaceDialog, CreateStage};
use crate::table::tests::fixture::{Fixture, cluster};

fn open_dialog(env: &mut Env) -> gpui::Entity<CreateNamespaceDialog> {
    env.f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let bounds = env
        .f
        .vcx
        .debug_bounds("resource-table-create")
        .expect("the New button is in the toolbar");
    env.f
        .vcx
        .simulate_click(bounds.center(), Default::default());
    env.f.settle();
    active(&mut env.f).expect("New opens the create dialog on a Namespaces table")
}

fn active(f: &mut Fixture) -> Option<gpui::Entity<CreateNamespaceDialog>> {
    let tabs = f.tabs.clone();
    f.vcx.update(|_, cx| {
        let tab = tabs.read(cx).tab(&cluster())?.clone();
        let workspace = tab.read(cx).workspace().clone();
        let layer = workspace.read(cx).modal_layer().clone();
        layer.read(cx).active_modal::<CreateNamespaceDialog>()
    })
}

fn creates(f: &Fixture) -> Vec<(bool, serde_json::Value)> {
    f.ports()
        .resources
        .mutating_calls()
        .into_iter()
        .map(|call| match call {
            ResourceCall::Create {
                options, object, ..
            } => (options.dry_run, object),
            other => panic!("unexpected write {other:?}"),
        })
        .collect()
}

fn fill(env: &mut Env, d: &gpui::Entity<CreateNamespaceDialog>, name: &str, labels: &str) {
    let d = d.clone();
    env.f.vcx.update(|window, cx| {
        d.update(cx, |d, cx| d.fill(name, labels, window, cx));
    });
    env.f.settle();
}

#[gpui::test]
fn new_on_the_namespaces_table_opens_the_create_dialog(cx: &mut TestAppContext) {
    let mut env = Env::standard(cx);
    let _d = open_dialog(&mut env);
    env.f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(env.f.vcx.debug_bounds("namespace-create-dialog").is_some());
    assert!(env.f.vcx.debug_bounds("namespace-create-name").is_some());
    assert!(env.f.vcx.debug_bounds("namespace-create-labels").is_some());
    assert!(
        creates(&env.f).is_empty(),
        "opening the dialog sends nothing"
    );
}

#[gpui::test]
fn invalid_names_and_labels_are_rejected_as_typed(cx: &mut TestAppContext) {
    let mut env = Env::standard(cx);
    let d = open_dialog(&mut env);
    for bad in ["Team A", "-a", "a_b", &"x".repeat(64)] {
        fill(&mut env, &d, bad, "");
        let (problem, can) =
            d.read_with(&env.f.vcx, |d, cx| (d.name_problem(cx), d.can_submit(cx)));
        assert!(problem.is_some(), "{bad:?} is refused");
        assert!(!can, "{bad:?} cannot be submitted");
    }
    env.f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        env.f
            .vcx
            .debug_bounds("namespace-create-name-problem")
            .is_some()
    );

    fill(&mut env, &d, "team-c", "bad label");
    let (labels, can) = d.read_with(&env.f.vcx, |d, cx| (d.labels_problem(cx), d.can_submit(cx)));
    assert!(labels.is_some() && !can);

    fill(&mut env, &d, "", "");
    assert!(
        !d.read_with(&env.f.vcx, |d, cx| d.can_submit(cx)),
        "empty name"
    );

    // Submitting what cannot go sends nothing.
    fill(&mut env, &d, "Bad Name", "");
    d.update(&mut env.f.vcx, |d, cx| d.submit(cx));
    env.f.settle();
    assert!(creates(&env.f).is_empty());
    assert!(env.f.state.audit_log().is_empty());
}

#[gpui::test]
fn a_valid_name_and_labels_create_the_namespace_through_the_guard(cx: &mut TestAppContext) {
    let mut env = Env::standard(cx);
    let d = open_dialog(&mut env);
    fill(&mut env, &d, "team-c", "team=c, env=dev");
    assert!(d.read_with(&env.f.vcx, |d, cx| d.can_submit(cx)));
    d.update(&mut env.f.vcx, |d, cx| d.submit(cx));
    env.f.settle();

    let sent = creates(&env.f);
    assert_eq!(sent.len(), 2, "a rehearsal, then the create: {sent:?}");
    assert!(sent[0].0 && !sent[1].0);
    assert_eq!(
        sent[1].1,
        json!({
            "apiVersion": "v1",
            "kind": "Namespace",
            "metadata": { "name": "team-c", "labels": { "env": "dev", "team": "c" } },
        })
    );
    let audit = env.f.state.audit_log();
    assert_eq!(audit.len(), 1);
    assert_eq!(&*audit[0].cmd, "namespace::Create");
    assert_eq!(&*audit[0].target.name, "team-c");
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(audit[0].initiator, Initiator::Ui);
    // The dialog closes with a toast.
    assert!(active(&mut env.f).is_none());
    assert!(
        toasts(&mut env.f)
            .iter()
            .any(|t| t.message.as_ref() == "Created namespace team-c"),
        "{:?}",
        toasts(&mut env.f)
    );
}

#[gpui::test]
fn a_name_that_exists_is_refused_and_the_dialog_stays_open(cx: &mut TestAppContext) {
    let mut env = Env::standard(cx);
    let d = open_dialog(&mut env);
    fill(&mut env, &d, "team-a", "");
    d.update(&mut env.f.vcx, |d, cx| d.submit(cx));
    env.f.settle();

    assert!(active(&mut env.f).is_some(), "the dialog stays for a fix");
    let (stage, error) = d.read_with(&env.f.vcx, |d, _| (d.stage(), d.error().map(str::to_owned)));
    assert_eq!(stage, CreateStage::Editing);
    assert!(
        error
            .expect("the refusal is shown")
            .contains("already exists")
    );
    assert_eq!(creates(&env.f).len(), 1, "only the rehearsal went out");
    // Typing again clears the message.
    let dialog = d.clone();
    env.f
        .vcx
        .update(|window, cx| dialog.update(cx, |d, cx| d.focus_name(window, cx)));
    env.f.vcx.simulate_input("x");
    env.f.settle();
    assert!(d.read_with(&env.f.vcx, |d, _| d.error().is_none()));
}

#[gpui::test]
fn a_read_only_cluster_says_so_instead_of_opening_the_dialog(cx: &mut TestAppContext) {
    let mut env = Env::standard(cx);
    env.f.sessions.set_read_only(&cluster(), true).unwrap();
    env.f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let bounds = env.f.vcx.debug_bounds("resource-table-create").unwrap();
    env.f
        .vcx
        .simulate_click(bounds.center(), Default::default());
    env.f.settle();
    assert!(active(&mut env.f).is_none());
    assert!(!toasts(&mut env.f).is_empty());
}

#[gpui::test]
fn other_tables_new_button_still_opens_the_resource_picker(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    f.ports().resources.insert(ns("team-a", json!({})));
    f.connect_with([crate::table::tests::p("team-a", "web-0", "1")]);
    let _pods = f.open_pods();
    f.dispatcher.clear();
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let bounds = f.vcx.debug_bounds("resource-table-create").unwrap();
    f.vcx.simulate_click(bounds.center(), Default::default());
    f.settle();
    assert!(active(&mut f).is_none());
    assert!(
        f.dispatcher
            .sent()
            .iter()
            .any(|c| matches!(c, oxikube_domain::command::Command::ResourceCreate { .. }))
    );
}
