//! `WorkloadService`: planning a scale and scaling, through the `workload::Scale` command.

use oxikube_domain::ErrorKind;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::view::WorkloadKind;
use oxikube_ports::{ListPage, WriteOptions};
use oxikube_testkit::ResourceCall;
use serde_json::json;

use super::{deployment, scale, scale_to, workload};
use crate::command_bus::Outcome;
use crate::testing::{Harness, ctx};
use crate::workloads::WorkloadService;

fn completed(outcome: Outcome) -> crate::command_bus::CommandOutput {
    match outcome {
        Outcome::Completed(output) => output,
        other @ Outcome::NeedsConfirmation(_) => panic!("expected completion, got {other:?}"),
    }
}

#[test]
fn scale_calls_the_scale_subresource_with_the_count() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    let resources = h.resources("a");
    resources.script().get_scale.push_ok(scale(3, 3));
    resources.script().scale.push_ok(scale(5, 3));

    let output = completed(
        h.confirm_and_run(scale_to(deployment("a"), 5), ctx(Initiator::Ui))
            .unwrap(),
    );
    assert_eq!(
        output.message.as_deref(),
        Some("Scaled Deployment shop/web from 3 to 5 replicas")
    );
    let data = output.data.unwrap();
    assert_eq!(
        (data["before"].as_u64(), data["after"].as_u64()),
        (Some(3), Some(5))
    );
    assert_eq!(data["dry_run"], false);

    // One write, to the subresource (not a patch of the object), for 5, not a dry run.
    let writes = h.writes("a");
    let [
        ResourceCall::Scale {
            kind,
            namespace,
            name,
            replicas,
            options,
        },
    ] = writes.as_slice()
    else {
        panic!("expected one scale write, got {writes:?}");
    };
    assert_eq!(kind, &deployment("a").gvk);
    assert_eq!(namespace.as_deref(), Some("shop"));
    assert_eq!((name.as_str(), *replicas), ("web", 5));
    assert_eq!(*options, WriteOptions::default());
}

#[test]
fn every_scalable_kind_scales_through_its_own_gvk() {
    for (group, kind) in [
        ("apps", "Deployment"),
        ("apps", "StatefulSet"),
        ("apps", "ReplicaSet"),
        ("", "ReplicationController"),
    ] {
        let h = Harness::with_workloads();
        h.connect("a", false);
        h.resources("a").script().get_scale.push_ok(scale(1, 1));
        h.resources("a").script().scale.push_ok(scale(2, 1));
        let target = workload("a", group, kind, "x");
        h.confirm_and_run(scale_to(target.clone(), 2), ctx(Initiator::Ui))
            .unwrap();
        let writes = h.writes("a");
        assert!(
            matches!(&writes[..], [ResourceCall::Scale { kind: k, replicas: 2, .. }] if *k == target.gvk),
            "{kind}: {writes:?}"
        );
    }
}

#[test]
fn a_daemonset_cannot_be_scaled_and_nothing_is_sent() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    let daemonset = workload("a", "apps", "DaemonSet", "agent");
    let error = h
        .confirm_and_run(scale_to(daemonset, 2), ctx(Initiator::Ui))
        .unwrap_err();
    let crate::command_bus::DispatchError::Handler(error) = error else {
        panic!("expected a handler error, got {error:?}");
    };
    assert_eq!(error.kind(), ErrorKind::Validation);
    assert!(error.message().contains("DaemonSet"), "{}", error.message());
    assert!(h.writes("a").is_empty());
    assert!(
        h.resources("a")
            .recorded_calls()
            .iter()
            .all(|call| !matches!(call, ResourceCall::GetScale { .. })),
        "not even the read"
    );
}

#[test]
fn a_dry_run_dispatch_has_the_server_rehearse_and_changes_nothing() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    h.resources("a").script().get_scale.push_ok(scale(3, 3));
    h.resources("a").script().scale.push_ok(scale(6, 3));
    let output = completed(
        h.confirm_and_run(
            scale_to(deployment("a"), 6),
            ctx(Initiator::Agent).with_dry_run(true),
        )
        .unwrap(),
    );
    assert_eq!(
        output.message.as_deref(),
        Some("Dry run: would scale Deployment shop/web from 3 to 6 replicas")
    );
    let writes = h.writes("a");
    assert!(
        matches!(&writes[..], [ResourceCall::Scale { replicas: 6, options, .. }] if options.dry_run),
        "{writes:?}"
    );
    assert!(h.audit()[0].dry_run);
}

#[test]
fn a_workload_that_is_gone_is_not_found_and_nothing_is_written() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    h.resources("a")
        .script()
        .get_scale
        .push_err(oxikube_domain::OxiError::not_found(
            "deployments.apps \"web\" not found",
        ));
    let error = h
        .confirm_and_run(scale_to(deployment("a"), 2), ctx(Initiator::Ui))
        .unwrap_err();
    let crate::command_bus::DispatchError::Handler(error) = error else {
        panic!("expected a handler error, got {error:?}");
    };
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(h.writes("a").is_empty());
}

#[test]
fn an_account_that_may_scale_but_not_read_the_scale_still_scales() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    h.resources("a")
        .script()
        .get_scale
        .push_err(oxikube_domain::OxiError::forbidden(
            "cannot get deployments/scale",
        ));
    h.resources("a").script().scale.push_ok(scale(4, 0));
    h.confirm_and_run(scale_to(deployment("a"), 4), ctx(Initiator::Ui))
        .unwrap();
    assert_eq!(h.writes("a").len(), 1);
    let detail = h.audit()[0].detail.clone().unwrap();
    assert!(detail.contains("before=unknown after=4"), "{detail}");
}

#[test]
fn plan_scale_reads_current_and_desired() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    // Desired 5, 3 exist (a scale-up in flight).
    h.resources("a").script().get_scale.push_ok(scale(5, 3));
    let service = WorkloadService::new(h.manager.clone());
    let plan = futures::executor::block_on(service.plan_scale(&deployment("a"))).unwrap();
    assert_eq!(plan.kind, WorkloadKind::Deployment);
    assert_eq!((plan.current, plan.desired), (3, 5));
    assert_eq!(
        plan.autoscaler, None,
        "no HorizontalPodAutoscaler list is a hint that is absent"
    );
    // Reading is not writing.
    assert!(h.writes("a").is_empty());
    assert!(h.audit().is_empty());
}

#[test]
fn plan_scale_refuses_what_cannot_scale_and_unknown_clusters() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    let service = WorkloadService::new(h.manager);
    let error = futures::executor::block_on(service.plan_scale(&workload(
        "a",
        "apps",
        "DaemonSet",
        "agent",
    )))
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Validation);
    let error = futures::executor::block_on(service.plan_scale(&deployment("b"))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound, "cluster b is not open");
}

#[test]
fn a_horizontal_pod_autoscaler_that_targets_the_workload_is_named() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    let hpa = |name: &str, target: &str| {
        oxikube_domain::Resource::from_json(json!({
            "apiVersion": "autoscaling/v2", "kind": "HorizontalPodAutoscaler",
            "metadata": {"name": name, "namespace": "shop"},
            "spec": {"scaleTargetRef": {"apiVersion": "apps/v1", "kind": "Deployment", "name": target},
                     "minReplicas": 2, "maxReplicas": 8}
        }))
        .unwrap()
    };
    let resources = h.resources("a");
    resources.script().list.push_ok(ListPage::complete(vec![
        hpa("other", "api"),
        hpa("web-hpa", "web"),
    ]));
    resources.script().get_scale.push_ok(scale(2, 2));
    let service = WorkloadService::new(h.manager);
    let plan = futures::executor::block_on(service.plan_scale(&deployment("a"))).unwrap();
    assert_eq!(plan.autoscaler.as_deref(), Some("web-hpa"));
}

#[test]
fn the_command_is_a_tool_stub_and_mutating() {
    let meta = oxikube_domain::command::lookup(oxikube_domain::command::CommandId::WORKLOAD_SCALE)
        .unwrap();
    assert!(meta.mutating);
    assert_eq!(meta.id.tool_name(), "k8s.workload_scale");
    let _ = Command::WorkloadScale {
        target: deployment("a"),
        replicas: 1,
    };
}
