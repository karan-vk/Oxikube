//! `workload::Restart`: the merge patch at the injected time, through the guard.

use std::time::Duration;

use oxikube_domain::ErrorKind;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_ports::PatchKind;
use oxikube_testkit::ResourceCall;
use serde_json::json;

use super::{deployment, restart_of, rollable, workload};
use crate::command_bus::DispatchError;
use crate::testing::{Harness, ctx};

const RESTARTED_AT: &str = "kubectl.kubernetes.io/restartedAt";

fn patches(h: &Harness) -> Vec<(bool, PatchKind, serde_json::Value)> {
    h.writes("a")
        .into_iter()
        .map(|call| match call {
            ResourceCall::Patch { patch, options, .. } => (options.dry_run, patch.kind, patch.body),
            other => panic!("expected a patch, got {other:?}"),
        })
        .collect()
}

#[test]
fn restart_sends_a_merge_patch_stamping_the_template_at_the_injected_time() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    h.resources("a").insert(rollable("Deployment", "web", None));
    h.clock.advance(Duration::from_secs(3725));

    let outcome = h
        .confirm_and_run(restart_of(deployment("a")), ctx(Initiator::Ui))
        .unwrap();
    let output = outcome.completed().unwrap();
    assert_eq!(
        output.message.as_deref(),
        Some("Restarting Deployment shop/web: new pods are rolling out")
    );
    let data = output.data.unwrap();
    assert_eq!(data["restarted_at"], "2026-01-01T01:02:05Z");
    assert_eq!(data["open"]["name"], "web", "a view is asked to open it");

    // The server rehearses the patch, then the real one; both are the same merge patch.
    let body = json!({"spec": {"template": {"metadata": {"annotations": {
        RESTARTED_AT: "2026-01-01T01:02:05Z"
    }}}}});
    assert_eq!(
        patches(&h),
        [
            (true, PatchKind::Merge, body.clone()),
            (false, PatchKind::Merge, body)
        ]
    );
    // It touched nothing else: the labels and containers are as they were.
    let live = &h.resources("a").objects()[0];
    assert_eq!(
        live.get_str("/spec/template/metadata/annotations/kubectl.kubernetes.io~1restartedAt"),
        Some("2026-01-01T01:02:05Z")
    );
    assert_eq!(
        live.get_str("/spec/template/metadata/labels/app"),
        Some("web")
    );
    assert_eq!(
        live.get_str("/spec/template/spec/containers/0/image"),
        Some("nginx:1.26")
    );
}

#[test]
fn a_template_with_annotations_keeps_them() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    h.resources("a")
        .insert(rollable("Deployment", "web", Some(json!({"team": "shop"}))));
    h.confirm_and_run(restart_of(deployment("a")), ctx(Initiator::Ui))
        .unwrap();
    let live = &h.resources("a").objects()[0];
    assert_eq!(
        live.get_str("/spec/template/metadata/annotations/team"),
        Some("shop")
    );
    assert!(
        live.get_str("/spec/template/metadata/annotations/kubectl.kubernetes.io~1restartedAt")
            .is_some()
    );
}

#[test]
fn a_workload_without_an_annotations_map_gets_one() {
    // The merge patch creates the missing map; nothing reads the object first.
    let h = Harness::with_workloads();
    h.connect("a", false);
    let bare = rollable("StatefulSet", "db", None);
    assert!(bare.get("/spec/template/metadata/annotations").is_none());
    h.resources("a").insert(bare);
    let target = workload("a", "apps", "StatefulSet", "db");
    h.confirm_and_run(restart_of(target), ctx(Initiator::Ui))
        .unwrap();
    let live = &h.resources("a").objects()[0];
    assert!(live.get("/spec/template/metadata/annotations").is_some());
}

#[test]
fn deployments_statefulsets_and_daemonsets_restart_and_the_others_are_refused() {
    for kind in ["Deployment", "StatefulSet", "DaemonSet"] {
        let h = Harness::with_workloads();
        h.connect("a", false);
        h.resources("a").insert(rollable(kind, "x", None));
        h.confirm_and_run(
            restart_of(workload("a", "apps", kind, "x")),
            ctx(Initiator::Ui),
        )
        .unwrap_or_else(|error| panic!("{kind}: {error}"));
        assert_eq!(patches(&h).len(), 2, "{kind}");
    }
    for (group, kind) in [
        ("apps", "ReplicaSet"),
        ("", "ReplicationController"),
        ("", "Pod"),
    ] {
        let h = Harness::with_workloads();
        h.connect("a", false);
        let error = h
            .confirm_and_run(
                restart_of(workload("a", group, kind, "x")),
                ctx(Initiator::Ui),
            )
            .unwrap_err();
        let DispatchError::Handler(error) = error else {
            panic!("{kind}: {error:?}");
        };
        assert_eq!(error.kind(), ErrorKind::Validation, "{kind}");
        assert!(error.message().contains("cannot be restarted"), "{kind}");
        assert!(h.writes("a").is_empty(), "{kind}");
    }
}

#[test]
fn restart_is_low_risk_with_one_simple_confirmation_for_every_initiator() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    for initiator in Initiator::ALL {
        let request = h.ask(restart_of(deployment("a")), ctx(initiator));
        assert_eq!(request.risk, Some(Risk::Low), "{initiator}");
        assert_eq!(request.tier, ConfirmTier::Simple, "{initiator}");
        assert!(
            request
                .summary
                .starts_with("Restart Deployment shop/web on "),
            "{}",
            request.summary
        );
    }
    assert!(
        h.writes("a").is_empty(),
        "nothing is sent before the answer"
    );
}

#[test]
fn a_read_only_cluster_refuses_every_initiator_before_any_request() {
    let h = Harness::with_workloads();
    h.connect("a", true);
    for initiator in Initiator::ALL {
        let error = h
            .dispatch(restart_of(deployment("a")), ctx(initiator))
            .unwrap_err();
        assert!(
            matches!(error, DispatchError::ReadOnly { .. }),
            "{initiator}"
        );
    }
    assert!(
        h.resources("a").recorded_calls().is_empty(),
        "no request of any kind"
    );
    assert!(h.audit().iter().all(|r| r.outcome == AuditOutcome::Denied));
}

#[test]
fn the_audit_record_names_the_initiator_the_time_and_the_servers_version() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    h.resources("a").insert(rollable("Deployment", "web", None));
    h.confirm_and_run(restart_of(deployment("a")), ctx(Initiator::Agent))
        .unwrap();
    let audit = h.audit();
    assert_eq!(audit.len(), 1);
    let record = &audit[0];
    assert_eq!(&*record.cmd, "workload::Restart");
    assert_eq!(record.target, deployment("a"));
    assert_eq!(record.initiator, Initiator::Agent);
    assert_eq!(record.outcome, AuditOutcome::Succeeded);
    // The fake keeps the object's own version; a real server returns the new one.
    assert_eq!(
        record.detail.as_deref(),
        Some("restartedAt=2026-01-01T00:00:00Z rv=7")
    );
}

#[test]
fn a_dry_run_dispatch_rehearses_once_and_changes_nothing() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    h.resources("a").insert(rollable("Deployment", "web", None));
    let output = h
        .confirm_and_run(
            restart_of(deployment("a")),
            ctx(Initiator::Agent).with_dry_run(true),
        )
        .unwrap()
        .completed()
        .unwrap();
    assert_eq!(
        output.message.as_deref(),
        Some("Dry run: Deployment shop/web can be restarted")
    );
    assert_eq!(output.data.unwrap()["open"], serde_json::Value::Null);
    let sent = patches(&h);
    assert_eq!(sent.len(), 1);
    assert!(sent[0].0, "the one request is a server dry run");
    assert!(
        h.resources("a").objects()[0]
            .get("/spec/template/metadata/annotations")
            .is_none(),
        "the stored object is unchanged"
    );
}

#[test]
fn a_refusal_by_the_server_dry_run_stops_before_the_real_patch() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    h.resources("a")
        .script()
        .patch
        .push_err(oxikube_domain::OxiError::forbidden("denied by admission"));
    let error = h
        .confirm_and_run(restart_of(deployment("a")), ctx(Initiator::Ui))
        .unwrap_err();
    assert!(matches!(error, DispatchError::Handler(_)));
    assert_eq!(patches(&h).len(), 1, "only the rehearsal was sent");
    assert_eq!(h.audit()[0].outcome, AuditOutcome::Failed);
}
