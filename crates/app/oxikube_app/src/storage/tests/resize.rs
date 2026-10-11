//! `pvc::Resize` through the bus and the guard.

use oxikube_domain::ErrorKind;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_ports::PatchKind;
use oxikube_testkit::ResourceCall;
use oxikube_testkit::storage::{pvc, storage_class};
use serde_json::json;

use super::{claim_ref, harness, script_patches, seed};
use crate::command_bus::{DispatchError, Outcome};
use crate::testing::{Harness, ctx};

fn resize(namespace: &str, size: &str, from: Option<&str>) -> Command {
    Command::PvcResize {
        target: claim_ref(namespace, "data"),
        size: size.into(),
        from: from.map(str::to_owned),
    }
}

fn expandable_cluster(h: &Harness) {
    seed(
        h,
        [
            pvc("data").namespace("shop").class("fast").build(),
            storage_class("fast").expandable(true).build(),
        ],
    );
}

fn patches(h: &Harness) -> Vec<ResourceCall> {
    h.writes(super::CLUSTER)
        .into_iter()
        .filter(|c| matches!(c, ResourceCall::Patch { .. }))
        .collect()
}

#[test]
fn a_resize_patches_the_requested_storage_after_a_dry_run() {
    let h = harness();
    expandable_cluster(&h);
    let grown = pvc("data")
        .namespace("shop")
        .class("fast")
        .requests("20Gi")
        .condition("FileSystemResizePending", "True", "Waiting for the kubelet")
        .build();
    script_patches(&h, &grown, 2);

    let outcome = h
        .confirm_and_run(resize("shop", "20Gi", Some("10Gi")), ctx(Initiator::Ui))
        .expect("the resize runs");
    let Outcome::Completed(output) = outcome else {
        panic!("confirmed, so completed");
    };
    let data = output.data.expect("structured result");
    assert_eq!(data["outcome"], "resized");
    assert_eq!(data["from"], "10Gi");
    assert_eq!(data["to"], "20Gi");
    assert_eq!(data["conditions"][0]["type"], "FileSystemResizePending");

    let calls = patches(&h);
    assert_eq!(calls.len(), 2, "a rehearsal, then the patch");
    let ResourceCall::Patch {
        kind,
        namespace,
        name,
        patch,
        options,
    } = &calls[1]
    else {
        unreachable!()
    };
    assert_eq!(kind.kind.as_ref(), "PersistentVolumeClaim");
    assert_eq!(namespace.as_deref(), Some("shop"));
    assert_eq!(name, "data");
    assert_eq!(patch.kind, PatchKind::Merge);
    assert_eq!(
        patch.body,
        json!({"spec": {"resources": {"requests": {"storage": "20Gi"}}}})
    );
    assert!(!options.dry_run);
    assert!(matches!(&calls[0], ResourceCall::Patch { options, .. } if options.dry_run));
}

#[test]
fn the_audit_record_has_the_old_and_the_new_size() {
    let h = harness();
    expandable_cluster(&h);
    script_patches(
        &h,
        &pvc("data").namespace("shop").requests("20Gi").build(),
        2,
    );
    h.confirm_and_run(resize("shop", "20Gi", Some("10Gi")), ctx(Initiator::Ui))
        .unwrap();
    let records = h.audit();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(&*record.cmd, "pvc::Resize");
    assert_eq!(record.outcome, AuditOutcome::Succeeded);
    assert_eq!(record.target.name.as_ref(), "data");
    assert_eq!(record.detail.as_deref(), Some("from=10Gi to=20Gi"));
}

#[test]
fn the_guard_asks_a_simple_confirmation_naming_the_claim_and_the_sizes() {
    let h = harness();
    expandable_cluster(&h);
    let request = h.ask(resize("shop", "20Gi", Some("10Gi")), ctx(Initiator::Ui));
    assert_eq!(request.command, CommandId::PVC_RESIZE);
    assert_eq!(request.tier, ConfirmTier::Simple);
    assert_eq!(request.risk, Some(Risk::Medium));
    assert!(
        request.summary.contains("shop/data") && request.summary.contains("from 10Gi to 20Gi"),
        "{}",
        request.summary
    );
    assert!(patches(&h).is_empty(), "nothing is sent before the answer");
}

#[test]
fn a_size_that_does_not_grow_is_refused_and_nothing_is_patched() {
    let h = harness();
    expandable_cluster(&h);
    for size in ["10Gi", "5Gi"] {
        let error = h
            .confirm_and_run(resize("shop", size, Some("10Gi")), ctx(Initiator::Ui))
            .expect_err("a shrink is refused");
        let DispatchError::Handler(error) = error else {
            panic!("a handler refusal, got {error:?}");
        };
        assert_eq!(error.kind(), ErrorKind::Validation);
        assert!(
            error.message().contains("must be greater"),
            "{}",
            error.message()
        );
    }
    assert!(patches(&h).is_empty());
    assert!(h.audit().iter().all(|r| r.outcome == AuditOutcome::Failed));
}

#[test]
fn a_class_that_forbids_expansion_blocks_with_that_reason() {
    let h = harness();
    seed(
        &h,
        [
            pvc("data").namespace("shop").class("fast").build(),
            storage_class("fast").build(),
        ],
    );
    let error = h
        .confirm_and_run(resize("shop", "20Gi", None), ctx(Initiator::Ui))
        .expect_err("expansion is not allowed");
    let DispatchError::Handler(error) = error else {
        panic!("a handler refusal");
    };
    assert!(
        error.message().contains("allowVolumeExpansion"),
        "{}",
        error.message()
    );
    assert!(patches(&h).is_empty());
}

#[test]
fn a_claim_with_no_class_cannot_be_resized() {
    let h = harness();
    seed(&h, [pvc("data").namespace("shop").no_class().build()]);
    let error = h
        .confirm_and_run(resize("shop", "20Gi", None), ctx(Initiator::Ui))
        .expect_err("no class");
    assert!(error.to_string().contains("no StorageClass"), "{error}");
    assert!(patches(&h).is_empty());
}

#[test]
fn a_class_that_cannot_be_read_does_not_block_the_server_decides() {
    let h = harness();
    // The claim names a class the cluster does not have (or the user may not read).
    seed(&h, [pvc("data").namespace("shop").class("gone").build()]);
    script_patches(
        &h,
        &pvc("data").namespace("shop").requests("20Gi").build(),
        2,
    );
    h.confirm_and_run(resize("shop", "20Gi", Some("10Gi")), ctx(Initiator::Ui))
        .expect("the server's own check is the gate");
    assert_eq!(patches(&h).len(), 2);
}

#[test]
fn the_server_refusal_in_the_dry_run_stops_before_the_real_patch() {
    let h = harness();
    expandable_cluster(&h);
    h.resources(super::CLUSTER)
        .script()
        .patch
        .push_err(oxikube_domain::OxiError::validation(
            "only dynamically provisioned claims can be resized",
        ));
    let error = h
        .confirm_and_run(resize("shop", "20Gi", Some("10Gi")), ctx(Initiator::Ui))
        .expect_err("the rehearsal fails");
    assert!(
        error.to_string().contains("dynamically provisioned"),
        "{error}"
    );
    assert_eq!(patches(&h).len(), 1, "only the rehearsal was sent");
    assert!(patches(&h).iter().all(|c| c.is_dry_run()));
}

#[test]
fn a_claim_that_changed_since_the_dialog_is_not_resized() {
    let h = harness();
    expandable_cluster(&h);
    // The dialog saw 5Gi; the claim is 10Gi now.
    let error = h
        .confirm_and_run(resize("shop", "20Gi", Some("5Gi")), ctx(Initiator::Ui))
        .expect_err("stale");
    let DispatchError::Handler(error) = error else {
        panic!("a handler refusal");
    };
    assert_eq!(error.kind(), ErrorKind::Conflict);
    assert!(patches(&h).is_empty());
}

#[test]
fn a_read_only_cluster_refuses_the_resize_for_every_initiator() {
    let h = Harness::with_extra(|registry, _, manager| {
        crate::storage::register_commands(registry, manager.clone())
    });
    h.connect(super::CLUSTER, true);
    expandable_cluster(&h);
    for initiator in [Initiator::Ui, Initiator::Command, Initiator::Agent] {
        let result = h.dispatch(resize("shop", "20Gi", Some("10Gi")), ctx(initiator));
        assert!(
            matches!(result, Err(DispatchError::ReadOnly { .. })),
            "{initiator:?}: {result:?}"
        );
    }
    assert!(h.writes(super::CLUSTER).is_empty());
    assert!(h.audit().iter().all(|r| r.outcome == AuditOutcome::Denied));
}

#[test]
fn a_dry_run_dispatch_rehearses_and_changes_nothing() {
    let h = harness();
    expandable_cluster(&h);
    script_patches(
        &h,
        &pvc("data").namespace("shop").requests("20Gi").build(),
        1,
    );
    let request = h.ask(
        resize("shop", "20Gi", Some("10Gi")),
        ctx(Initiator::Agent).with_dry_run(true),
    );
    let answer = crate::guard::Confirmation::simple(request.token);
    let outcome = h
        .dispatch(
            resize("shop", "20Gi", Some("10Gi")),
            ctx(Initiator::Agent)
                .with_dry_run(true)
                .with_confirmation(answer),
        )
        .unwrap();
    let Outcome::Completed(output) = outcome else {
        panic!("completed");
    };
    assert_eq!(output.data.unwrap()["outcome"], "dry_run");
    assert_eq!(patches(&h).len(), 1);
    assert!(patches(&h)[0].is_dry_run());
}

#[test]
fn only_a_claim_can_be_resized() {
    let h = harness();
    let wrong = Command::PvcResize {
        target: super::class_ref("fast"),
        size: "20Gi".into(),
        from: None,
    };
    let error = h
        .confirm_and_run(wrong, ctx(Initiator::Ui))
        .expect_err("not a claim");
    assert!(
        error.to_string().contains("PersistentVolumeClaim"),
        "{error}"
    );
}
