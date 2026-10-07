//! `pod::Debug` (E09-S10) through the guard: a low-risk mutation with a simple confirmation that
//! says the container cannot be removed, blocked in read-only mode for everyone, audited with the
//! image and the target, and run end to end by the [`DebugRunner`] over a real service.

use std::sync::Arc;

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_domain::{ErrorKind, OxiError, Resource};
use oxikube_testkit::ExecPortCall;
use serde_json::json;

use crate::command_bus::{CommandOutput, DispatchError, HandlerContext, Outcome};
use crate::exec::{DebugRequest, DebugRunner, ExecService};
use crate::testing::{Harness, ctx, declared, id, pod};

fn debug(image: &str) -> Command {
    Command::PodDebug {
        target: pod("a", "web-0"),
        image: image.into(),
        target_container: Some("app".into()),
        command: vec!["sh".into(), "-c".into(), "echo hunter2".into()],
        name: None,
    }
}

fn web() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "web-0", "namespace": "default", "uid": "u1"},
        "spec": {"containers": [{"name": "app", "image": "distroless"}]},
        "status": {"phase": "Running", "containerStatuses": [
            {"name": "app", "ready": true, "restartCount": 0, "image": "distroless",
             "state": {"running": {"startedAt": "2026-10-01T00:00:00Z"}}}]},
    }))
    .unwrap()
}

#[test]
fn a_debug_container_asks_for_a_simple_confirmation_that_says_it_is_permanent() {
    let h = Harness::with_every_command();
    h.connect("a", false);
    h.allow_deletes("a", 1);
    let asked = h.ask(debug("busybox"), ctx(Initiator::Ui));
    assert_eq!(asked.command, CommandId::POD_DEBUG);
    assert_eq!(asked.tier, ConfirmTier::Simple);
    assert_eq!(asked.risk, Some(Risk::Low));
    assert!(
        asked.expected_name.is_none(),
        "no typing for a low-risk action"
    );
    for part in ["busybox", "container app", "web-0", "cannot be removed"] {
        assert!(asked.summary.contains(part), "{part}: {}", asked.summary);
    }
    assert!(h.calls().is_empty(), "nothing ran before the answer");

    let done = h
        .confirm_and_run(debug("busybox"), ctx(Initiator::Ui))
        .unwrap();
    assert!(matches!(done, Outcome::Completed(_)));
    let calls = h.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].mutation, "the handler got the guard's permit");
}

#[test]
fn a_read_only_cluster_blocks_it_for_every_initiator_and_audits_the_refusal() {
    let h = Harness::with_every_command();
    h.connect("a", true);
    for initiator in Initiator::ALL {
        let err = h.dispatch(debug("busybox"), ctx(initiator)).unwrap_err();
        assert!(
            matches!(err, DispatchError::ReadOnly { .. }),
            "{initiator}: {err:?}"
        );
        assert_eq!(OxiError::from(err).kind(), ErrorKind::Forbidden);
    }
    assert!(h.calls().is_empty());
    let audit = h.audit();
    assert_eq!(audit.len(), Initiator::ALL.len());
    for record in &audit {
        assert_eq!(record.outcome, AuditOutcome::Denied);
        let detail = record.detail.as_deref().unwrap_or("");
        assert!(
            detail.contains("image=busybox") && detail.contains("target=app"),
            "{detail}"
        );
    }
}

#[test]
fn the_audit_record_names_the_image_the_target_and_the_program_never_its_arguments() {
    let h = Harness::with_every_command();
    h.connect("a", false);
    h.allow_deletes("a", 1);
    h.confirm_and_run(debug("nicolaka/netshoot"), ctx(Initiator::Ui))
        .unwrap();
    let audit = h.audit();
    let [record] = audit.as_slice() else {
        panic!("one record, got {audit:?}");
    };
    assert_eq!(&*record.cmd, "pod::Debug");
    assert_eq!(record.outcome, AuditOutcome::Succeeded);
    assert_eq!(record.initiator, Initiator::Ui);
    assert_eq!(record.target, pod("a", "web-0"));
    assert_eq!(
        record.detail.as_deref(),
        Some("session=debug image=nicolaka/netshoot target=app program=sh")
    );
    assert!(!serde_json::to_string(record).unwrap().contains("hunter2"));

    // Declining is audited as cancelled, with the same detail.
    let asked = h.ask(debug("busybox"), ctx(Initiator::Ui));
    futures::FutureExt::now_or_never(h.bus.decline(asked.token))
        .expect("no wait")
        .expect("declined");
    let last = h.audit().pop().unwrap();
    assert_eq!(last.outcome, AuditOutcome::Cancelled);
    assert!(
        last.detail
            .as_deref()
            .unwrap_or("")
            .contains("image=busybox")
    );
}

/// A harness whose `pod::Debug` handler is the one the terminal registers, minus the tab: it opens
/// the debug container through a real `ExecService` under the guard's permit.
fn real() -> (Harness, Arc<ExecService>, DebugRunner) {
    let service_slot = Arc::new(parking_lot::Mutex::new(None::<Arc<ExecService>>));
    let slot = service_slot.clone();
    let h = Harness::with_extra(move |reg, _, manager| {
        let service = Arc::new(ExecService::new(manager.clone()));
        *slot.lock() = Some(service.clone());
        reg.register(
            declared(CommandId::POD_DEBUG),
            move |command: Command, cx: HandlerContext| {
                let service = service.clone();
                async move {
                    let request = DebugRequest::from_command(&command)?;
                    let opened = service.open_debug(cx.require_mutation()?, &request).await?;
                    Ok(CommandOutput {
                        message: Some(format!("debug container {} is running", opened.plan.name)),
                        data: Some(json!({ "container": opened.plan.name })),
                    })
                }
            },
        )
    });
    let service = service_slot.lock().clone().expect("registered");
    let runner = DebugRunner::new(h.bus.clone(), "alice");
    (h, service, runner)
}

fn request() -> DebugRequest {
    let mut request = DebugRequest::new(pod("a", "web-0"), "busybox");
    request.target_container = Some("app".into());
    request
}

#[tokio::test]
async fn the_runner_confirms_for_the_dialog_and_the_container_is_added_under_the_guard() {
    let (h, _service, runner) = real();
    h.connect("a", false);
    let ports = h.connector.ports_for(&id("a"));
    ports.resources.script().get.push_ok(web());
    let report = runner.run(&request()).await.unwrap();
    assert!(report.container.starts_with("debugger-"), "{report:?}");
    assert!(report.message.contains(&report.container));
    let calls = ports.exec.recorded_calls();
    let [ExecPortCall::CreateDebugContainer(spec)] = calls.as_slice() else {
        panic!("one debug container, got {calls:?}");
    };
    assert_eq!(spec.image, "busybox");
    assert_eq!(spec.target_container.as_deref(), Some("app"));
    assert_eq!(spec.name.as_deref(), Some(report.container.as_str()));
    let audit = h.audit();
    let [record] = audit.as_slice() else {
        panic!("one record");
    };
    assert_eq!(
        (record.outcome, record.initiator),
        (AuditOutcome::Succeeded, Initiator::Ui)
    );
    assert_eq!(h.bus.guard().pending_confirmations(), 0);
}

#[tokio::test]
async fn the_runner_cannot_get_past_read_only_and_reports_a_rejected_patch() {
    let (h, _service, runner) = real();
    h.connect("a", true);
    let err = runner.run(&request()).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Forbidden, "{err}");
    assert!(
        h.connector
            .ports_for(&id("a"))
            .exec
            .recorded_calls()
            .is_empty()
    );

    let (h, _service, runner) = real();
    h.connect("a", false);
    let ports = h.connector.ports_for(&id("a"));
    ports.resources.script().get.push_ok(web());
    ports
        .exec
        .script()
        .create_debug_container
        .push_err(OxiError::forbidden(
            "pods \"web-0\" is forbidden: User cannot patch resource \"pods/ephemeralcontainers\"",
        ));
    let err = runner.run(&request()).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Forbidden);
    assert!(err.message().contains("pods/ephemeralcontainers"), "{err}");
    assert_eq!(h.audit().last().unwrap().outcome, AuditOutcome::Failed);

    // A blank image never leaves the dialog.
    let mut blank = request();
    blank.image = "  ".into();
    let err = runner.run(&blank).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
}
