//! The two dialogs' flows: what they read, what they refuse and what they send.

use futures::FutureExt as _;
use oxikube_domain::ErrorKind;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::view::storage::{Expansion, ResizeBlock};
use oxikube_testkit::ResourceCall;
use oxikube_testkit::storage::{pvc, storage_class};

use super::{CLUSTER, claim_ref, class_ref, harness, script_patches, seed};
use crate::storage::{DefaultClassFlow, ResizeFlow};
use crate::testing::Harness;

fn resize_flow(h: &Harness) -> ResizeFlow {
    ResizeFlow::new(h.bus.clone(), h.manager.clone(), "alice")
}

fn default_flow(h: &Harness) -> DefaultClassFlow {
    DefaultClassFlow::new(h.bus.clone(), h.manager.clone(), "alice")
}

#[test]
fn opening_a_resize_reads_the_claim_and_its_class() {
    let h = harness();
    seed(
        &h,
        [
            pvc("data")
                .namespace("shop")
                .class("fast")
                .requests("10Gi")
                .build(),
            storage_class("fast").expandable(true).build(),
        ],
    );
    let context = resize_flow(&h)
        .open(&claim_ref("shop", "data"))
        .now_or_never()
        .expect("fakes answer at once")
        .expect("opens");
    assert_eq!(context.current().unwrap().to_string(), "10Gi");
    assert_eq!(context.expansion, Expansion::Allowed);
    assert_eq!(context.blocked(), None);
    assert_eq!(context.check("20Gi").unwrap().to_string(), "20Gi");
    assert!(matches!(
        context.check("10Gi"),
        Err(ResizeBlock::NotLarger { .. })
    ));
    assert!(matches!(context.check("x"), Err(ResizeBlock::Invalid(_))));
    assert!(h.writes(CLUSTER).is_empty(), "opening writes nothing");
}

#[test]
fn a_class_that_forbids_expansion_blocks_before_anything_is_typed() {
    let h = harness();
    seed(
        &h,
        [
            pvc("data").namespace("shop").class("slow").build(),
            storage_class("slow").build(),
        ],
    );
    let context = resize_flow(&h)
        .open(&claim_ref("shop", "data"))
        .now_or_never()
        .unwrap()
        .unwrap();
    assert_eq!(
        context.blocked(),
        Some(ResizeBlock::ExpansionDisallowed {
            class: "slow".into()
        })
    );
    // Even a good size is refused while the class forbids it.
    assert!(context.check("50Gi").is_err());
}

#[test]
fn opening_a_missing_claim_is_not_found() {
    let h = harness();
    let error = resize_flow(&h)
        .open(&claim_ref("shop", "nope"))
        .now_or_never()
        .unwrap()
        .expect_err("no such claim");
    assert_eq!(error.kind(), ErrorKind::NotFound);
}

#[test]
fn running_a_resize_sends_the_command_as_the_user_and_reports_the_conditions() {
    let h = harness();
    seed(
        &h,
        [
            pvc("data").namespace("shop").class("fast").build(),
            storage_class("fast").expandable(true).build(),
        ],
    );
    script_patches(
        &h,
        &pvc("data")
            .namespace("shop")
            .class("fast")
            .requests("20Gi")
            .condition("FileSystemResizePending", "True", "Waiting for the kubelet")
            .build(),
        // The guard's dry run before it asks (E19-S07), the handler's, the patch.
        3,
    );
    let flow = resize_flow(&h);
    let context = flow
        .open(&claim_ref("shop", "data"))
        .now_or_never()
        .unwrap()
        .unwrap();
    let size = context.check("20Gi").unwrap();
    let report = flow
        .run(&context, size)
        .now_or_never()
        .unwrap()
        .expect("the resize runs");
    assert_eq!(report.to.to_string(), "20Gi");
    assert_eq!(report.from.unwrap().to_string(), "10Gi");
    assert_eq!(&*report.conditions[0].kind, "FileSystemResizePending");
    assert_eq!(
        report.conditions[0].message.as_deref(),
        Some("Waiting for the kubelet")
    );
    let records = h.audit();
    assert_eq!(records.len(), 2, "the guard's dry run, then the resize");
    assert!(records[0].dry_run);
    let records = &records[1..];
    assert_eq!(records[0].initiator, oxikube_domain::audit::Initiator::Ui);
    assert_eq!(records[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(records[0].detail.as_deref(), Some("from=10Gi to=20Gi"));
}

#[test]
fn a_claim_that_changed_under_the_dialog_is_a_conflict() {
    let h = harness();
    seed(
        &h,
        [
            pvc("data").namespace("shop").class("fast").build(),
            storage_class("fast").expandable(true).build(),
        ],
    );
    let flow = resize_flow(&h);
    let context = flow
        .open(&claim_ref("shop", "data"))
        .now_or_never()
        .unwrap()
        .unwrap();
    // Somebody else resizes it to 15Gi while the dialog is open.
    seed(
        &h,
        [pvc("data")
            .namespace("shop")
            .class("fast")
            .requests("15Gi")
            .build()],
    );
    let size = context.check("20Gi").unwrap();
    let error = flow
        .run(&context, size)
        .now_or_never()
        .unwrap()
        .expect_err("stale");
    assert_eq!(error.kind(), ErrorKind::Conflict);
    assert!(
        h.writes(CLUSTER)
            .iter()
            .all(|c| !matches!(c, ResourceCall::Patch { .. }))
    );
}

#[test]
fn a_read_only_cluster_refuses_the_flow() {
    let h = Harness::with_extra(|registry, _, manager| {
        crate::storage::register_commands(registry, manager.clone())
    });
    h.connect(CLUSTER, true);
    seed(
        &h,
        [
            pvc("data").namespace("shop").class("fast").build(),
            storage_class("fast").expandable(true).build(),
        ],
    );
    let flow = resize_flow(&h);
    let context = flow
        .open(&claim_ref("shop", "data"))
        .now_or_never()
        .unwrap()
        .unwrap();
    let size = context.check("20Gi").unwrap();
    let error = flow
        .run(&context, size)
        .now_or_never()
        .unwrap()
        .expect_err("read-only");
    assert_eq!(error.kind(), ErrorKind::Forbidden);
    assert!(h.writes(CLUSTER).is_empty());
}

#[test]
fn opening_the_default_dialog_lists_the_previous_defaults() {
    let h = harness();
    seed(
        &h,
        [
            storage_class("old").default_class().build(),
            storage_class("older").beta_default_class().build(),
            storage_class("fast").build(),
        ],
    );
    let context = default_flow(&h)
        .open(&class_ref("fast"))
        .now_or_never()
        .unwrap()
        .unwrap();
    assert!(!context.class.is_default);
    let mut previous: Vec<String> = context.previous.iter().map(|n| n.to_string()).collect();
    previous.sort();
    assert_eq!(previous, ["old", "older"]);
    assert!(!context.is_noop());
    let only = default_flow(&h)
        .open(&class_ref("old"))
        .now_or_never()
        .unwrap()
        .unwrap();
    assert_eq!(only.previous.len(), 1, "older is the other default");
}

#[test]
fn running_set_default_reports_what_was_cleared() {
    let h = harness();
    seed(
        &h,
        [
            storage_class("old").default_class().build(),
            storage_class("fast").build(),
        ],
    );
    script_patches(&h, &storage_class("fast").default_class().build(), 4);
    let flow = default_flow(&h);
    let context = flow
        .open(&class_ref("fast"))
        .now_or_never()
        .unwrap()
        .unwrap();
    let report = flow
        .run(&context, true)
        .now_or_never()
        .unwrap()
        .expect("runs");
    assert_eq!(report.cleared, ["old"]);
    assert!(report.message.contains("fast"), "{}", report.message);
}
