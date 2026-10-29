//! Stop on error (on and off) and cancel mid-run: what is left is reported skipped.

use futures::StreamExt as _;
use futures::executor::block_on;
use oxikube_domain::{OxiError, Resource};

use super::*;

fn invalid() -> OxiError {
    OxiError::validation("ConfigMap \"bad\" is invalid")
}

#[test]
fn without_stop_on_error_a_failed_dry_run_fails_alone() {
    let t = Applying::new(false);
    let plan = t.plan(vec![configmap("bad", None), configmap("good", None)]);
    t.port().script().patch.push_err(invalid());
    let report = t
        .apply_plan(&plan, &ApplyOptions::default(), Answering::confirm())
        .expect("ran");
    assert_eq!(words(&report), ["failed", "created"]);
    assert_eq!(t.applied_names(), ["good"]);
}

#[test]
fn with_stop_on_error_a_failed_dry_run_applies_nothing() {
    let t = Applying::new(false);
    let plan = t.plan(vec![
        namespace("shop"),
        configmap("bad", None),
        configmap("good", None),
    ]);
    let ns = Resource::from_json(namespace("shop")).unwrap();
    t.port().script().patch.push_ok(ns);
    t.port().script().patch.push_err(invalid());
    let options = ApplyOptions {
        stop_on_error: true,
        ..ApplyOptions::default()
    };
    let report = t
        .apply_plan(&plan, &options, Answering::confirm())
        .expect("ran");
    assert_eq!(words(&report), ["skipped", "failed", "skipped"]);
    assert_eq!(
        reason(&report.objects[0]),
        "not applied: ConfigMap default/bad failed its dry run and stop on error is set"
    );
    assert!(
        t.writes().iter().all(|(_, dry_run)| *dry_run),
        "nothing written"
    );
}

#[test]
fn with_stop_on_error_a_failed_apply_skips_the_later_ranks() {
    let t = Applying::new(false);
    let plan = t.plan(vec![
        namespace("shop"),
        configmap("a", None),
        deployment("web"),
    ]);
    // The plan's dry run passes; the namespace's own write fails.
    for object in [
        namespace("shop"),
        configmap("a", Some("default")),
        deployment("web"),
    ] {
        t.port()
            .script()
            .patch
            .push_ok(Resource::from_json(object).unwrap());
    }
    t.port()
        .script()
        .patch
        .push_err(OxiError::forbidden("namespaces is forbidden"));
    let options = ApplyOptions {
        stop_on_error: true,
        ..ApplyOptions::default()
    };
    let report = t
        .apply_plan(&plan, &options, Answering::confirm())
        .expect("ran");
    assert_eq!(words(&report), ["failed", "skipped", "skipped"]);
    assert_eq!(
        reason(&report.objects[1]),
        "not sent: an earlier object failed and stop on error is set"
    );
    assert!(t.applied_names().is_empty());
}

#[test]
fn a_cancel_mid_run_skips_every_object_not_yet_sent() {
    let t = Applying::new(false);
    // Each write yields once, so the run can be stopped between objects.
    t.port().track_write_overlap();
    let plan = t.plan(vec![
        namespace("shop"),
        configmap("a", None),
        deployment("web"),
        deployment("api"),
    ]);
    let options = ApplyOptions::default();
    let mut stream = t.service.apply_plan(&plan, &options, Answering::confirm());
    let first = block_on(stream.next()).expect("an item").expect("a result");
    assert_eq!(first.label, "Namespace shop");
    options.cancel.cancel();
    let rest: Vec<ObjectResult> = block_on(stream.collect::<Vec<_>>())
        .into_iter()
        .map(|item| item.expect("a result"))
        .collect();
    let mut report = PlanReport { objects: rest };
    report.objects.insert(0, first);
    report.objects.sort_by_key(|o| o.index);
    // The ConfigMap was already on its way when the cancel came; the deployments were not.
    assert_eq!(words(&report), ["created", "created", "skipped", "skipped"]);
    assert_eq!(reason(&report.objects[2]), "cancelled: not sent");
    assert_eq!(t.applied_names(), ["shop", "a"]);
}

#[test]
fn a_cancel_before_the_dry_run_rehearses_nothing() {
    let t = Applying::new(false);
    let plan = t.plan(vec![configmap("a", None)]);
    let options = ApplyOptions::default();
    options.cancel.cancel();
    let report = t.dry_run_plan(&plan, &options).expect("ran");
    assert_eq!(words(&report), ["skipped"]);
    assert!(t.writes().is_empty());
}
