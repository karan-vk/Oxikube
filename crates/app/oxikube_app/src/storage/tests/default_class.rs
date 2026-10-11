//! `storageclass::SetDefault` through the bus and the guard.

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_testkit::ResourceCall;
use oxikube_testkit::storage::storage_class;
use serde_json::json;

use super::{class_ref, harness, script_patches, seed};
use crate::command_bus::{DispatchError, Outcome};
use crate::testing::{Harness, ctx};

const DEFAULT: &str = "storageclass.kubernetes.io/is-default-class";
const BETA: &str = "storageclass.beta.kubernetes.io/is-default-class";

fn set_default(name: &str, clear_previous: bool) -> Command {
    Command::StorageClassSetDefault {
        target: class_ref(name),
        clear_previous,
    }
}

/// `(name, annotation patch, dry run)` of every patch sent, in order.
fn patches(h: &Harness) -> Vec<(String, serde_json::Value, bool)> {
    h.writes(super::CLUSTER)
        .into_iter()
        .filter_map(|call| match call {
            ResourceCall::Patch {
                name,
                patch,
                options,
                ..
            } => Some((
                name,
                patch.body["metadata"]["annotations"].clone(),
                options.dry_run,
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn setting_the_default_annotates_the_class_and_clears_the_previous_one() {
    let h = harness();
    seed(
        &h,
        [
            storage_class("old").default_class().build(),
            storage_class("fast").build(),
            storage_class("cold").build(),
        ],
    );
    script_patches(&h, &storage_class("fast").default_class().build(), 4);
    let outcome = h
        .confirm_and_run(set_default("fast", true), ctx(Initiator::Ui))
        .unwrap();
    let Outcome::Completed(output) = outcome else {
        panic!("completed");
    };
    let data = output.data.unwrap();
    assert_eq!(data["outcome"], "set");
    assert_eq!(data["set"], "fast");
    assert_eq!(data["cleared"], json!(["old"]));

    // Each write is rehearsed first; the new default is set before the old one is cleared.
    assert_eq!(
        patches(&h),
        [
            ("fast".to_owned(), json!({DEFAULT: "true"}), true),
            ("old".to_owned(), json!({DEFAULT: "false"}), true),
            ("fast".to_owned(), json!({DEFAULT: "true"}), false),
            ("old".to_owned(), json!({DEFAULT: "false"}), false),
        ]
    );
    let record = &h.audit()[0];
    assert_eq!(&*record.cmd, "storageclass::SetDefault");
    assert_eq!(record.outcome, AuditOutcome::Succeeded);
    assert_eq!(record.detail.as_deref(), Some("clear_previous=true"));
}

#[test]
fn the_previous_default_can_be_left_alone() {
    let h = harness();
    seed(
        &h,
        [
            storage_class("old").default_class().build(),
            storage_class("fast").build(),
        ],
    );
    script_patches(&h, &storage_class("fast").default_class().build(), 2);
    h.confirm_and_run(set_default("fast", false), ctx(Initiator::Ui))
        .unwrap();
    let names: Vec<_> = patches(&h).into_iter().map(|(n, ..)| n).collect();
    assert_eq!(names, ["fast", "fast"], "the old default is untouched");
}

#[test]
fn a_legacy_beta_default_is_dropped_when_it_is_cleared() {
    let h = harness();
    seed(
        &h,
        [
            storage_class("old").beta_default_class().build(),
            storage_class("fast").build(),
        ],
    );
    script_patches(&h, &storage_class("fast").default_class().build(), 4);
    h.confirm_and_run(set_default("fast", true), ctx(Initiator::Ui))
        .unwrap();
    let cleared = patches(&h)
        .into_iter()
        .find(|(name, _, dry)| name == "old" && !dry)
        .unwrap();
    assert_eq!(cleared.1, json!({DEFAULT: "false", BETA: null}));
}

#[test]
fn a_class_that_is_the_only_default_is_left_alone() {
    let h = harness();
    seed(&h, [storage_class("fast").default_class().build()]);
    let outcome = h
        .confirm_and_run(set_default("fast", true), ctx(Initiator::Ui))
        .unwrap();
    let Outcome::Completed(output) = outcome else {
        panic!("completed");
    };
    assert_eq!(output.data.unwrap()["outcome"], "unchanged");
    assert!(patches(&h).is_empty());
}

#[test]
fn the_guard_asks_a_simple_low_risk_confirmation() {
    let h = harness();
    seed(&h, [storage_class("fast").build()]);
    let request = h.ask(set_default("fast", true), ctx(Initiator::Ui));
    assert_eq!(request.command, CommandId::STORAGECLASS_SET_DEFAULT);
    assert_eq!(request.tier, ConfirmTier::Simple);
    assert_eq!(request.risk, Some(Risk::Low));
    assert!(request.summary.contains("fast"), "{}", request.summary);
}

#[test]
fn an_unknown_class_is_not_found() {
    let h = harness();
    seed(&h, [storage_class("fast").build()]);
    let error = h
        .confirm_and_run(set_default("missing", true), ctx(Initiator::Ui))
        .expect_err("no such class");
    assert!(error.to_string().contains("missing"), "{error}");
    assert!(patches(&h).is_empty());
}

#[test]
fn a_read_only_cluster_refuses_it_for_every_initiator() {
    let h = Harness::with_extra(|registry, _, manager| {
        crate::storage::register_commands(registry, manager.clone())
    });
    h.connect(super::CLUSTER, true);
    seed(&h, [storage_class("fast").build()]);
    for initiator in [Initiator::Ui, Initiator::Command, Initiator::Agent] {
        let result = h.dispatch(set_default("fast", true), ctx(initiator));
        assert!(
            matches!(result, Err(DispatchError::ReadOnly { .. })),
            "{result:?}"
        );
    }
    assert!(h.writes(super::CLUSTER).is_empty());
}

#[test]
fn a_dry_run_rehearses_and_writes_nothing() {
    let h = harness();
    seed(&h, [storage_class("fast").build()]);
    script_patches(&h, &storage_class("fast").default_class().build(), 1);
    let request = h.ask(
        set_default("fast", true),
        ctx(Initiator::Agent).with_dry_run(true),
    );
    let answer = crate::guard::Confirmation::simple(request.token);
    h.dispatch(
        set_default("fast", true),
        ctx(Initiator::Agent)
            .with_dry_run(true)
            .with_confirmation(answer),
    )
    .unwrap();
    assert_eq!(patches(&h).len(), 1);
    assert!(patches(&h)[0].2, "a rehearsal only");
}
