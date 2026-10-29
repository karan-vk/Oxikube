//! What each object did: created, configured, unchanged; a conflict with other managers and the
//! forced re-run; a failure that skips only what needs it.

use oxikube_domain::{ConflictDetails, ConflictReason, FieldCause, OxiError, Resource};
use oxikube_ports::PatchKind;
use oxikube_testkit::ResourceCall;

use super::*;

fn live(name: &str, mode: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": name, "namespace": "default", "resourceVersion": "5"},
        "data": {"mode": mode}
    }))
    .expect("a valid object")
}

#[test]
fn each_object_is_created_configured_or_unchanged() {
    let t = Applying::new(false);
    t.port().insert(live("same", "blue"));
    t.port().insert(live("changed", "green"));
    let plan = t.plan(vec![
        configmap("new", None),
        configmap("same", None),
        configmap("changed", None),
    ]);
    let report = t
        .apply_plan(&plan, &ApplyOptions::default(), Answering::confirm())
        .expect("applied");
    assert_eq!(words(&report), ["created", "unchanged", "configured"]);
    assert_eq!(report.summary(), "1 created, 1 configured, 1 unchanged");
    assert!(
        report
            .objects
            .iter()
            .all(|o| !o.dry_run && o.method.is_some())
    );
}

#[test]
fn the_server_answer_decides_unchanged_by_its_version() {
    let t = Applying::new(false);
    t.port().insert(live("same", "green"));
    // The server kept the object as it was: the same resourceVersion, though the manifest's mode
    // differs from what the store holds.
    let kept = live("same", "green");
    t.port().script().patch.push_ok(kept.clone()); // the plan's dry run
    t.port().script().patch.push_ok(kept.clone()); // the object's own rehearsal
    t.port().script().patch.push_ok(kept); // the apply
    let plan = t.plan(vec![configmap("same", None)]);
    let report = t
        .apply_plan(&plan, &ApplyOptions::default(), Answering::confirm())
        .expect("applied");
    assert_eq!(words(&report), ["unchanged"]);
}

fn conflict() -> OxiError {
    OxiError::conflict("Apply failed with 1 conflict").with_source(ConflictDetails {
        reason: ConflictReason::FieldOwnership,
        causes: vec![FieldCause {
            field: ".data.mode".into(),
            manager: Some("kubectl".into()),
            reason: "FieldManagerConflict".into(),
            message: "conflict with kubectl".into(),
        }],
    })
}

#[test]
fn a_conflict_names_the_managers_and_only_a_forced_run_takes_the_fields() {
    let t = Applying::new(false);
    t.port().insert(live("web", "green"));
    t.port().script().patch.push_err(conflict());
    let plan = t.plan(vec![configmap("web", None)]);
    let report = t
        .dry_run_plan(&plan, &ApplyOptions::default())
        .expect("dry run");
    let ObjectStatus::Failed { conflicts, .. } = &report.objects[0].status else {
        panic!("a conflict fails the object: {:?}", report.objects[0]);
    };
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].manager.as_deref(), Some("kubectl"));
    assert_eq!(conflicts[0].field, ".data.mode");

    // Without force the apply sends nothing for it.
    t.port().script().patch.push_err(conflict());
    let report = t
        .apply_plan(&plan, &ApplyOptions::default(), Answering::confirm())
        .expect("ran");
    assert_eq!(words(&report), ["failed"]);
    assert!(t.writes().iter().all(|(_, dry_run)| *dry_run));

    // The user chose force for this run: every request carries it.
    let forced = ApplyOptions {
        force_conflicts: true,
        ..ApplyOptions::default()
    };
    let report = t
        .apply_plan(&plan, &forced, Answering::confirm())
        .expect("applied");
    assert_eq!(words(&report), ["configured"]);
    let real: Vec<_> =
        t.h.writes("a")
            .into_iter()
            .filter(|call| !call.is_dry_run())
            .collect();
    assert!(matches!(
        &real[..],
        [ResourceCall::Patch { patch, .. }] if patch.kind == PatchKind::Apply {
            manager: "oxikube".into(),
            force: true
        }
    ));
}

#[test]
fn a_failed_namespace_skips_what_lives_in_it_and_nothing_else() {
    let t = Applying::new(false);
    let plan = t.plan(vec![
        namespace("shop"),
        configmap("in-shop", Some("shop")),
        configmap("elsewhere", None),
    ]);
    // The plan's dry run passes for all three; the namespace's own write is refused.
    let ns = Resource::from_json(namespace("shop")).unwrap();
    t.port().script().patch.push_ok(ns);
    t.port().script().patch.push_ok(live("in-shop", "-"));
    t.port().script().patch.push_ok(live("elsewhere", "-"));
    t.port()
        .script()
        .patch
        .push_err(OxiError::forbidden("namespaces is forbidden"));
    let report = t
        .apply_plan(&plan, &ApplyOptions::default(), Answering::confirm())
        .expect("ran");
    assert_eq!(words(&report), ["failed", "skipped", "created"]);
    assert_eq!(
        reason(&report.objects[1]),
        "not sent: it needs Namespace shop, which did not apply"
    );
    assert_eq!(t.applied_names(), ["elsewhere"]);
    assert_eq!(report.failures(), 2);
}

#[test]
fn a_rejected_field_is_named() {
    let t = Applying::new(false);
    t.port().script().patch.push_err(
        OxiError::validation("ConfigMap \"web\" is invalid").with_source(
            oxikube_domain::ValidationDetails {
                causes: vec![FieldCause {
                    field: "data.mode".into(),
                    manager: None,
                    reason: "FieldValueInvalid".into(),
                    message: "bad".into(),
                }],
            },
        ),
    );
    let plan = t.plan(vec![configmap("web", None)]);
    let report = t
        .dry_run_plan(&plan, &ApplyOptions::default())
        .expect("dry run");
    let ObjectStatus::Failed { fields, reason, .. } = &report.objects[0].status else {
        panic!("failed");
    };
    assert_eq!(fields, &["data.mode"]);
    assert!(reason.contains("is invalid"), "{reason}");
}
