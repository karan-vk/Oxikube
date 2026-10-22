//! `helm::Rollback` and `helm::Uninstall` through the real guard over `FakeHelmPort`.

mod diff;

use std::sync::Arc;

use oxikube_domain::audit::{AuditOutcome, AuditRecord, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_testkit::{FakeHelmPort, HelmCall};

use super::register_commands;
use crate::command_bus::{DispatchContext, DispatchError, Outcome};
use crate::guard::Confirmation;
use crate::testing::{Harness, ctx, id};

struct Setup {
    h: Harness,
    helm: Arc<FakeHelmPort>,
}

impl Setup {
    fn new(read_only: bool) -> Self {
        let helm = Arc::new(FakeHelmPort::new());
        let port = helm.clone();
        let h = Harness::build(&[], move |reg, _, _| register_commands(reg, port));
        h.connect("a", read_only);
        Self { h, helm }
    }

    fn audit_of(&self, command: CommandId) -> Vec<AuditRecord> {
        self.h
            .audit()
            .into_iter()
            .filter(|r| &*r.cmd == command.as_str())
            .collect()
    }

    fn context(&self, initiator: Initiator) -> DispatchContext {
        ctx(initiator).with_cluster(id("a"))
    }
}

fn rollback(revision: u32) -> Command {
    Command::HelmRollback {
        cluster: id("a"),
        namespace: "demo".into(),
        release: "web".into(),
        revision,
    }
}

fn uninstall(keep_history: bool) -> Command {
    Command::HelmUninstall {
        cluster: id("a"),
        namespace: "demo".into(),
        release: "web".into(),
        keep_history,
    }
}

#[test]
fn rollback_asks_one_simple_confirmation_then_restores_the_revision() {
    let s = Setup::new(false);
    let ctx = s.context(Initiator::Ui);
    let asked = s.h.ask(rollback(2), ctx.clone());
    assert_eq!(asked.tier, ConfirmTier::Simple);
    assert_eq!(asked.risk, Some(Risk::Medium));
    assert!(asked.expected_name.is_none());
    assert!(
        asked.summary.contains("release web") && asked.summary.contains("revision 2"),
        "{}",
        asked.summary
    );
    // Asking changes nothing.
    assert!(s.helm.recorded_calls().is_empty());

    s.helm.script().rollback.push_ok(());
    let outcome =
        s.h.dispatch(
            rollback(2),
            ctx.with_confirmation(Confirmation::simple(asked.token)),
        )
        .expect("rolled back");
    let Outcome::Completed(output) = outcome else {
        panic!("not completed: {outcome:?}")
    };
    assert_eq!(output.data.unwrap()["outcome"], "rolled_back");
    let calls = s.helm.recorded_calls();
    assert!(
        matches!(&calls[..], [HelmCall::Rollback(r)]
            if r.revision == 2 && !r.dry_run && r.release.name == "web" && r.release.namespace == "demo"),
        "{calls:?}"
    );
    let audit = s.audit_of(CommandId::HELM_ROLLBACK);
    let last = audit.last().unwrap();
    assert_eq!(last.outcome, AuditOutcome::Succeeded);
    assert_eq!(last.detail.as_deref(), Some("revision=2"));
    assert_eq!(&*last.target.name, "web");
    assert_eq!(last.target.namespace(), Some("demo"));
    assert_eq!(&*last.target.gvk.kind, "Release");
}

#[test]
fn uninstall_needs_the_release_name_typed() {
    let s = Setup::new(false);
    let ctx = s.context(Initiator::Ui);
    let asked = s.h.ask(uninstall(true), ctx.clone());
    assert_eq!(asked.tier, ConfirmTier::TypeName);
    assert_eq!(asked.risk, Some(Risk::High));
    assert_eq!(asked.expected_name.as_deref(), Some("web"));
    assert!(
        asked.summary.contains("keeping its history"),
        "{}",
        asked.summary
    );

    // Typing the wrong name does not run it.
    let wrong = s.h.dispatch(
        uninstall(true),
        ctx.clone()
            .with_confirmation(Confirmation::typed(asked.token, "other")),
    );
    assert!(
        matches!(wrong, Err(DispatchError::Confirmation(_))),
        "{wrong:?}"
    );
    assert!(s.helm.recorded_calls().is_empty());

    s.helm.script().uninstall.push_ok(());
    let asked = s.h.ask(uninstall(true), ctx.clone());
    s.h.dispatch(
        uninstall(true),
        ctx.with_confirmation(Confirmation::typed(asked.token, "web")),
    )
    .expect("uninstalled");
    let calls = s.helm.recorded_calls();
    assert!(
        matches!(&calls[..], [HelmCall::Uninstall(r)] if r.keep_history && !r.dry_run),
        "{calls:?}"
    );
    let last = s.audit_of(CommandId::HELM_UNINSTALL).pop().unwrap();
    assert_eq!(last.outcome, AuditOutcome::Succeeded);
    assert_eq!(last.detail.as_deref(), Some("keep_history=true"));
}

#[test]
fn uninstall_without_keep_history_removes_the_history_too() {
    let s = Setup::new(false);
    s.helm.script().uninstall.push_ok(());
    s.h.confirm_and_run(uninstall(false), s.context(Initiator::Ui))
        .expect("uninstalled");
    assert!(matches!(
        &s.helm.recorded_calls()[..],
        [HelmCall::Uninstall(r)] if !r.keep_history
    ));
}

#[test]
fn a_read_only_cluster_blocks_both_for_every_initiator() {
    let s = Setup::new(true);
    for initiator in [Initiator::Ui, Initiator::Agent] {
        for command in [rollback(1), uninstall(false)] {
            let denied = s.h.dispatch(command, s.context(initiator));
            assert!(
                matches!(denied, Err(DispatchError::ReadOnly { .. })),
                "{initiator:?}: {denied:?}"
            );
        }
    }
    assert!(s.helm.recorded_calls().is_empty());
    let denied =
        s.h.audit()
            .iter()
            .filter(|r| r.outcome == AuditOutcome::Denied)
            .count();
    assert_eq!(denied, 4);
}

#[test]
fn a_dry_run_asks_helm_to_rehearse_and_changes_nothing() {
    let s = Setup::new(false);
    let dry = s.context(Initiator::Ui).with_dry_run(true);
    s.helm.script().rollback.push_ok(());
    s.helm.script().uninstall.push_ok(());
    for command in [rollback(3), uninstall(false)] {
        let outcome =
            s.h.confirm_and_run(command, dry.clone())
                .expect("rehearsed");
        let Outcome::Completed(output) = outcome else {
            panic!("not completed")
        };
        assert_eq!(output.data.unwrap()["outcome"], "dry_run");
    }
    let calls = s.helm.recorded_calls();
    assert!(
        matches!(&calls[0], HelmCall::Rollback(r) if r.dry_run),
        "{calls:?}"
    );
    assert!(matches!(&calls[1], HelmCall::Uninstall(r) if r.dry_run));
    assert!(
        s.audit_of(CommandId::HELM_ROLLBACK)
            .iter()
            .all(|r| r.dry_run)
    );
}

#[test]
fn a_helm_failure_is_the_commands_error_and_is_audited() {
    let s = Setup::new(false);
    s.helm
        .script()
        .rollback
        .push_err(oxikube_domain::OxiError::internal(
            "release has no revision 9",
        ));
    let failed = s.h.confirm_and_run(rollback(9), s.context(Initiator::Ui));
    assert!(failed.is_err(), "{failed:?}");
    assert_eq!(
        s.audit_of(CommandId::HELM_ROLLBACK).last().unwrap().outcome,
        AuditOutcome::Failed
    );
}

#[test]
fn both_are_commands_with_tool_stubs() {
    for (id, tool, risk) in [
        (CommandId::HELM_ROLLBACK, "helm.rollback", Risk::Medium),
        (CommandId::HELM_UNINSTALL, "helm.uninstall", Risk::High),
    ] {
        let meta = oxikube_domain::command::lookup(id).expect("declared");
        assert!(meta.mutating);
        assert_eq!(meta.risk, Some(risk));
        assert_eq!(id.tool_name(), tool);
    }
}
