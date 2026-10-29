//! A custom resource and the definition the plan makes for it: deferred at the dry run, applied
//! once the definition is `Established` and discovery refreshed, skipped when it never is.

use std::time::Duration;

use futures::executor::LocalPool;
use futures::task::LocalSpawnExt as _;
use oxikube_domain::{OxiError, Resource};
use oxikube_testkit::DiscoveryCall;

use super::*;

fn established() -> Resource {
    let mut definition = crd();
    definition["status"] = json!({"conditions": [
        {"type": "NamesAccepted", "status": "True"},
        {"type": "Established", "status": "True"}
    ]});
    Resource::from_json(definition).unwrap()
}

/// The server cannot rehearse a Widget before its definition exists.
fn no_kind() -> OxiError {
    OxiError::not_found("no matches for kind \"Widget\" in version \"example.com/v1\"")
}

#[test]
fn a_custom_resource_whose_definition_is_in_the_plan_is_deferred_not_failed() {
    let t = Applying::new(false);
    let plan = t.plan(vec![widget("w"), crd()]);
    assert_eq!(plan.plan.items[1].depends_on, [0], "the CRD ranks first");
    assert_eq!(
        plan.plan.items[1].doc.json["metadata"]["namespace"], "default",
        "the definition in the plan says the kind is namespaced"
    );
    t.port().script().patch.push_ok(established());
    t.port().script().patch.push_err(no_kind());
    let report = t
        .dry_run_plan(&plan, &ApplyOptions::default())
        .expect("dry run");
    assert_eq!(words(&report), ["created", "deferred"]);
    assert_eq!(report.objects[1].status, ObjectStatus::DeferredDryRun);
}

#[test]
fn a_custom_resource_waits_for_its_definition_to_be_established() {
    let t = Applying::new(false);
    let plan = t.plan(vec![crd(), widget("w")]);
    // The plan's dry run: the definition rehearses, the widget cannot yet.
    t.port().script().patch.push_ok(established());
    t.port().script().patch.push_err(no_kind());
    let mut pool = LocalPool::new();
    let stream = t
        .service
        .apply_plan(&plan, &ApplyOptions::default(), Answering::confirm());
    let done = pool
        .spawner()
        .spawn_local_with_handle(stream.into_report())
        .unwrap();
    pool.run_until_stalled();
    // The definition is applied (the fake stores it without status) and polled: not established.
    assert_eq!(t.applied_names(), ["widgets.example.com"]);
    assert_eq!(t.h.clock.pending_sleepers(), 1, "waiting between polls");
    // The API server establishes it; the next poll sees it.
    t.port().insert(established());
    t.h.clock.advance(Duration::from_millis(500));
    pool.run_until_stalled();
    let report = pool.run_until(done).expect("applied");
    assert_eq!(words(&report), ["created", "created"]);
    assert_eq!(t.applied_names(), ["widgets.example.com", "w"]);
    let discovery = t.h.connector.ports_for(&id("a")).discovery.recorded_calls();
    assert!(
        discovery.contains(&DiscoveryCall::Discover),
        "discovery refreshed before the widget: {discovery:?}"
    );
}

#[test]
fn a_definition_never_established_skips_its_custom_resources_after_the_timeout() {
    let t = Applying::new(false);
    let plan = t.plan(vec![crd(), widget("w"), configmap("plain", None)]);
    // The plan's dry run, in plan order: the ConfigMap, the definition, the widget.
    let plain = Resource::from_json(configmap("plain", Some("default"))).unwrap();
    t.port().script().patch.push_ok(plain);
    t.port().script().patch.push_ok(established());
    t.port().script().patch.push_err(no_kind());
    let mut pool = LocalPool::new();
    let done = pool
        .spawner()
        .spawn_local_with_handle(
            t.service
                .apply_plan(&plan, &ApplyOptions::default(), Answering::confirm())
                .into_report(),
        )
        .unwrap();
    pool.run_until_stalled();
    let mut polls = 0;
    while t.h.clock.advance_to_next().is_some() {
        pool.run_until_stalled();
        polls += 1;
    }
    assert_eq!(polls, 60, "30 s at one poll every 500 ms");
    let report = pool.run_until(done).expect("ran");
    // Plan order: ConfigMap (rank 3), CRD (rank 5), Widget (custom, last).
    assert_eq!(words(&report), ["created", "created", "skipped"]);
    assert_eq!(
        reason(&report.objects[2]),
        "not sent: it needs CustomResourceDefinition widgets.example.com, which was not \
         Established in time"
    );
}

#[test]
fn a_definition_that_is_already_established_is_not_waited_for() {
    let t = Applying::new(false);
    t.port().insert(established());
    let plan = t.plan(vec![crd(), widget("w")]);
    // The fake stores the applied body over it (no status); the server would keep the status.
    t.port().script().patch.push_ok(established());
    t.port()
        .script()
        .patch
        .push_ok(Resource::from_json(widget("w")).unwrap());
    t.port().script().patch.push_ok(established());
    t.port().script().patch.push_ok(established());
    let report = t
        .apply_plan(&plan, &ApplyOptions::default(), Answering::confirm())
        .expect("applied");
    assert_eq!(words(&report), ["unchanged", "created"]);
    assert!(t.h.clock.recorded_calls().is_empty(), "no sleep");
}

#[test]
fn a_kind_the_cluster_does_not_serve_fails_unless_the_plan_defines_it() {
    let t = Applying::new(false);
    // The adapter answers a kind it cannot find in discovery with `Unsupported`.
    let unserved = || OxiError::unsupported("the cluster does not serve example.com/v1/Widget");
    t.port().script().patch.push_err(unserved());
    t.port().script().create.push_err(unserved());
    let alone = t.plan(vec![widget("w")]);
    let report = t
        .dry_run_plan(&alone, &ApplyOptions::default())
        .expect("dry run");
    assert_eq!(
        words(&report),
        ["failed"],
        "not a missing preview: {report:?}"
    );

    t.port().script().patch.push_ok(established());
    t.port().script().patch.push_err(unserved());
    t.port().script().create.push_err(unserved());
    let with_definition = t.plan(vec![crd(), widget("w")]);
    let report = t
        .dry_run_plan(&with_definition, &ApplyOptions::default())
        .expect("dry run");
    assert_eq!(words(&report), ["created", "deferred"]);
}
