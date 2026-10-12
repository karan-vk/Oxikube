//! `pod::Sanitize`: only Completed and Failed pods are deleted, one audit record per pod under a
//! shared batch id, an aggregated confirmation with the count, and the partial-failure report.

use oxikube_domain::OxiError;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::Command;
use oxikube_domain::safety::ConfirmTier;
use oxikube_testkit::ResourceCall;

use super::*;
use crate::command_bus::DispatchError;

fn sanitize(targets: &[&str]) -> Command {
    Command::PodSanitize {
        cluster: id("a"),
        namespace: Some("default".into()),
        targets: targets.iter().map(|name| pod_ref(name)).collect(),
    }
}

/// Four pods: one Completed, one Failed, one Running, one Pending.
fn with_pods() -> Workloads {
    let t = Workloads::new(false);
    for (name, phase) in [
        ("done", "Succeeded"),
        ("broken", "Failed"),
        ("web-0", "Running"),
        ("waiting", "Pending"),
    ] {
        t.port().insert(pod(name, phase));
    }
    t
}

fn names(t: &Workloads) -> Vec<String> {
    let mut names: Vec<_> = t
        .objects_of(&Gvk::new("", "v1", "Pod"))
        .iter()
        .map(|p| p.name().to_owned())
        .collect();
    names.sort();
    names
}

fn deleted_by_the_port(t: &Workloads) -> Vec<(String, bool)> {
    t.port()
        .mutating_calls()
        .into_iter()
        .filter_map(|call| match call {
            ResourceCall::Delete { name, options, .. } => Some((name, options.dry_run)),
            _ => None,
        })
        .collect()
}

#[test]
fn only_completed_and_failed_pods_of_the_view_are_deleted() {
    let t = with_pods();
    // The view lists all four (it drew a moment ago); the cluster decides.
    let out = t
        .run(
            sanitize(&["done", "broken", "web-0", "waiting"]),
            Initiator::Ui,
        )
        .expect("sanitize");
    assert_eq!(names(&t), ["waiting", "web-0"]);
    assert_eq!(out.message.as_deref(), Some("Deleted 2 pods, 2 skipped"));
    let data = out.data.unwrap();
    assert_eq!(
        (data["deleted"].as_u64(), data["skipped"].as_u64()),
        (Some(2), Some(2))
    );
    // The running and the pending pod never saw a delete request, not even a rehearsal.
    let sent = deleted_by_the_port(&t);
    assert!(
        sent.iter()
            .all(|(name, _)| name == "done" || name == "broken")
    );
    assert_eq!(sent.len(), 4, "a rehearsal and a delete for each: {sent:?}");
}

#[test]
fn a_pod_that_started_again_since_the_view_drew_is_left_alone() {
    let t = Workloads::new(false);
    t.port().insert(pod("flapping", "Running"));
    let out = t
        .run(sanitize(&["flapping"]), Initiator::Ui)
        .expect("sanitize");
    assert_eq!(out.message.as_deref(), Some("Deleted 0 pods, 1 skipped"));
    assert!(t.port().mutating_calls().is_empty());
}

#[test]
fn the_delete_is_bound_to_the_uid_that_was_read() {
    let t = with_pods();
    t.run(sanitize(&["done", "broken"]), Initiator::Ui)
        .expect("sanitize");
    let sent: Vec<_> = t
        .port()
        .mutating_calls()
        .into_iter()
        .filter_map(|call| match call {
            ResourceCall::Delete { name, options, .. } => Some((name, options)),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 4, "a rehearsal and a delete for each: {sent:?}");
    for (name, options) in sent {
        let uid = options.preconditions.and_then(|p| p.uid);
        assert_eq!(
            uid,
            Some(format!("{name}-uid")),
            "{name}, dry_run={}",
            options.dry_run
        );
    }
}

#[test]
fn a_pod_replaced_since_it_was_read_is_skipped_not_deleted() {
    let t = Workloads::new(false);
    t.port().insert(pod("web-0", "Failed"));
    // The server refuses the UID precondition: the StatefulSet made a new `web-0` since the read.
    t.port().script().delete.push_err(OxiError::conflict(
        "the UID in the precondition does not match",
    ));
    let out = t
        .run(sanitize(&["web-0"]), Initiator::Ui)
        .expect("sanitize");
    assert_eq!(out.message.as_deref(), Some("Deleted 0 pods, 1 skipped"));
    assert_eq!(names(&t), ["web-0"], "the replacement is still there");
    assert_eq!(deleted_by_the_port(&t), [("web-0".to_owned(), true)]);
    let audit = t.h.audit();
    let record = audit
        .iter()
        .find(|r| r.cmd.as_ref() == "pod::Delete")
        .expect("the attempt is on record");
    assert_eq!(record.outcome, AuditOutcome::Succeeded);
}

#[test]
fn the_fake_server_refuses_a_namesake_with_another_uid() {
    let t = Workloads::new(false);
    t.port().insert(pod("web-0", "Failed"));
    let options = oxikube_ports::DeleteOptions {
        preconditions: Some(oxikube_ports::Preconditions {
            uid: Some("old-uid".into()),
            resource_version: None,
        }),
        ..Default::default()
    };
    let result = futures::executor::block_on(oxikube_ports::ResourceWriter::delete(
        &*t.port(),
        &Gvk::new("", "v1", "Pod"),
        Some("default"),
        "web-0",
        &options,
    ));
    assert_eq!(
        result.unwrap_err().kind(),
        oxikube_domain::ErrorKind::Conflict
    );
    assert_eq!(names(&t), ["web-0"]);
}

#[test]
fn every_pod_gets_its_own_audit_record_under_one_batch_id() {
    let t = with_pods();
    let out = t
        .run(sanitize(&["done", "broken", "web-0"]), Initiator::Agent)
        .expect("sanitize");
    let batch = out.data.unwrap()["batch"].as_str().unwrap().to_owned();
    assert!(batch.starts_with("sweep-"), "{batch}");

    let audit = t.h.audit();
    let per_pod: Vec<_> = audit
        .iter()
        .filter(|r| r.cmd.as_ref() == "pod::Delete")
        .collect();
    assert_eq!(per_pod.len(), 2, "the running pod has no record: {audit:?}");
    let mut targets: Vec<_> = per_pod.iter().map(|r| r.target.name.to_string()).collect();
    targets.sort();
    assert_eq!(targets, ["broken", "done"]);
    for record in &per_pod {
        assert_eq!(record.initiator, Initiator::Agent);
        assert_eq!(record.outcome, AuditOutcome::Succeeded);
        assert_eq!(
            record.detail.as_deref(),
            Some(format!("batch={batch} via=pod::Sanitize").as_str())
        );
        assert!(!record.dry_run);
    }
    // The command itself is one record too, naming the batch and the size of the sweep.
    let umbrella = audit
        .iter()
        .find(|r| r.cmd.as_ref() == "pod::Sanitize")
        .unwrap();
    assert_eq!(umbrella.outcome, AuditOutcome::Succeeded);
    assert_eq!(
        umbrella.detail.as_deref(),
        Some(format!("batch={batch} pods=3").as_str())
    );
    assert_eq!(
        &*umbrella.target.name, "default",
        "recorded against the namespace"
    );
}

#[test]
fn without_targets_the_cluster_is_asked_for_the_finished_pods() {
    let t = with_pods();
    t.port().insert(
        Resource::from_json(json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": {"name": "elsewhere", "namespace": "other"},
            "status": {"phase": "Succeeded"},
        }))
        .unwrap(),
    );
    let command = Command::PodSanitize {
        cluster: id("a"),
        namespace: Some("default".into()),
        targets: Vec::new(),
    };
    let out = t.run(command, Initiator::Agent).expect("sanitize");
    assert_eq!(out.message.as_deref(), Some("Deleted 2 pods"));
    assert_eq!(names(&t), ["elsewhere", "waiting", "web-0"]);

    // Every namespace.
    let all = Command::PodSanitize {
        cluster: id("a"),
        namespace: None,
        targets: Vec::new(),
    };
    let out = t.run(all, Initiator::Agent).expect("sanitize");
    assert_eq!(out.message.as_deref(), Some("Deleted 1 pod"));
    assert_eq!(names(&t), ["waiting", "web-0"]);
}

#[test]
fn nothing_to_sweep_is_not_an_error() {
    let t = Workloads::new(false);
    let command = Command::PodSanitize {
        cluster: id("a"),
        namespace: None,
        targets: Vec::new(),
    };
    let out = t.run(command, Initiator::Ui).expect("sanitize");
    assert_eq!(
        out.message.as_deref(),
        Some("No Completed or Failed pods to delete")
    );
    assert!(t.port().mutating_calls().is_empty());
}

#[test]
fn the_confirmation_names_the_count_at_the_tier_of_a_pod_delete() {
    let t = with_pods();
    let ctx = DispatchContext::new(Initiator::Ui, "alice");
    let request = t.h.ask(sanitize(&["done", "broken"]), ctx.clone());
    assert_eq!(request.tier, ConfirmTier::Simple);
    assert!(
        request
            .summary
            .contains("Delete 2 Completed and Failed pods"),
        "{}",
        request.summary
    );
    let request = t.h.ask(sanitize(&["done"]), ctx.clone());
    assert!(
        request.summary.contains("Delete 1 Completed or Failed pod"),
        "{}",
        request.summary
    );
    let all = Command::PodSanitize {
        cluster: id("a"),
        namespace: Some("default".into()),
        targets: Vec::new(),
    };
    let request = t.h.ask(all, ctx);
    assert!(
        request
            .summary
            .contains("every Completed and Failed pod in namespace default"),
        "{}",
        request.summary
    );
    assert!(t.port().recorded_calls().is_empty());
}

#[test]
fn one_failure_does_not_stop_the_rest_and_is_reported_per_pod() {
    let t = with_pods();
    // `done`: the rehearsal and the delete; `broken`: its rehearsal is refused. The fakes never
    // wait, so the pods go one after the other in the order asked.
    let port = t.port();
    port.script()
        .delete
        .push_ok(oxikube_ports::DeleteOutcome::Deleted);
    port.script()
        .delete
        .push_ok(oxikube_ports::DeleteOutcome::Deleted);
    port.script()
        .delete
        .push_err(OxiError::forbidden("pods \"broken\" is forbidden"));
    let error = t
        .run(sanitize(&["done", "broken"]), Initiator::Ui)
        .unwrap_err();
    let DispatchError::Handler(error) = error else {
        panic!("{error:?}");
    };
    let text = error.message();
    assert!(text.contains("Deleted 1 pod, 1 failed"), "{text}");
    assert!(text.contains("broken: "), "{text}");

    let audit = t.h.audit();
    let outcome_of = |name: &str| {
        audit
            .iter()
            .find(|r| r.cmd.as_ref() == "pod::Delete" && &*r.target.name == name)
            .map(|r| r.outcome)
    };
    assert_eq!(outcome_of("done"), Some(AuditOutcome::Succeeded));
    assert_eq!(outcome_of("broken"), Some(AuditOutcome::Failed));
    let umbrella = audit
        .iter()
        .find(|r| r.cmd.as_ref() == "pod::Sanitize")
        .unwrap();
    assert_eq!(umbrella.outcome, AuditOutcome::Failed);
}

#[test]
fn a_dry_run_rehearses_every_delete_and_removes_nothing() {
    let t = with_pods();
    let out = t.dry_run(sanitize(&["done", "broken"])).expect("dry run");
    assert_eq!(
        out.message.as_deref(),
        Some("Dry run: 2 pods can be deleted")
    );
    assert!(deleted_by_the_port(&t).iter().all(|(_, dry)| *dry));
    assert_eq!(names(&t).len(), 4);
    assert!(t.h.audit().iter().all(|r| r.dry_run));
}

#[test]
fn pods_of_another_cluster_are_refused() {
    let t = with_pods();
    let mut stray = pod_ref("done");
    stray.cluster = id("b");
    let command = Command::PodSanitize {
        cluster: id("a"),
        namespace: None,
        targets: vec![stray],
    };
    let error = t.run(command, Initiator::Ui).unwrap_err();
    assert!(matches!(error, DispatchError::Handler(_)), "{error:?}");
    assert!(t.port().mutating_calls().is_empty());
}

#[test]
fn a_pod_named_twice_is_swept_once() {
    let t = with_pods();
    let out = t
        .run(sanitize(&["done", "done"]), Initiator::Ui)
        .expect("sanitize");
    assert_eq!(out.message.as_deref(), Some("Deleted 1 pod"));
    let swept =
        t.h.audit()
            .iter()
            .filter(|r| r.cmd.as_ref() == "pod::Delete")
            .count();
    assert_eq!(swept, 1, "one record, one delete");
}
