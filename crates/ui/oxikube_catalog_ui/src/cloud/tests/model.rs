//! The model: events in, phases and rows out.

use jiff::Timestamp;
use oxikube_app::cloud::CloudDiscoveryEvent;
use oxikube_domain::OxiError;
use oxikube_domain::cloud::{CloudDetection, CloudProvider, CloudScope};

use super::{degraded, ready, run_events};
use crate::cloud::test_support::{aws_scope, eks, managed};
use crate::cloud::{CloudModel, Issue, Phase};

fn now() -> Timestamp {
    Timestamp::from_second(1_800_000_000).expect("a valid timestamp")
}

#[test]
fn a_provider_is_hidden_until_its_cli_is_detected() {
    let mut model = CloudModel::new();
    assert_eq!(model.visible().count(), 0);
    assert!(model.selected().is_none());

    model.apply(
        &CloudDiscoveryEvent::Detected(CloudDetection::not_installed(CloudProvider::Aws)),
        now(),
    );
    model.apply(
        &CloudDiscoveryEvent::Detected(ready(CloudProvider::Gcp)),
        now(),
    );
    let visible: Vec<_> = model.visible().map(|p| p.provider).collect();
    assert_eq!(
        visible,
        [CloudProvider::Gcp],
        "AWS is not installed, Azure unknown"
    );
    assert_eq!(
        model.selected().map(|p| p.provider),
        Some(CloudProvider::Gcp)
    );
}

#[test]
fn a_run_goes_loading_with_progress_then_loaded() {
    let scope = aws_scope("prod", "eu-west-1");
    let other = aws_scope("prod", "us-east-1");
    let mut model = CloudModel::new();
    model.apply(
        &CloudDiscoveryEvent::Detected(ready(CloudProvider::Aws)),
        now(),
    );
    assert_eq!(model.provider(CloudProvider::Aws).phase(), Phase::Loaded);

    model.apply(
        &CloudDiscoveryEvent::Started {
            provider: CloudProvider::Aws,
            scopes: vec![scope.clone(), other],
        },
        now(),
    );
    assert_eq!(
        model.provider(CloudProvider::Aws).phase(),
        Phase::Loading { done: 0, total: 2 }
    );
    model.apply(
        &CloudDiscoveryEvent::ScopeDone {
            scope: scope.clone(),
            clusters: vec![eks(&scope, "web")],
        },
        now(),
    );
    assert_eq!(
        model.provider(CloudProvider::Aws).phase(),
        Phase::Loading { done: 1, total: 2 }
    );
    assert_eq!(
        model.provider(CloudProvider::Aws).clusters.len(),
        1,
        "shown as they arrive"
    );
    model.apply(
        &CloudDiscoveryEvent::Finished {
            provider: CloudProvider::Aws,
            clusters: vec![eks(&scope, "web")],
            failed_scopes: 0,
        },
        now(),
    );
    let state = model.provider(CloudProvider::Aws);
    assert_eq!(state.phase(), Phase::Loaded);
    assert_eq!(
        state.fetched_at,
        Some(now()),
        "a listing that ran is stamped"
    );
}

#[test]
fn a_cache_answer_keeps_its_own_age_and_a_stale_one_keeps_loading() {
    let scope = aws_scope("prod", "eu-west-1");
    let cached_at = Timestamp::from_second(1_799_999_000).expect("a valid timestamp");
    let mut model = CloudModel::new();
    model.apply(
        &CloudDiscoveryEvent::Cached {
            provider: CloudProvider::Aws,
            detection: Some(ready(CloudProvider::Aws)),
            clusters: vec![eks(&scope, "web")],
            fetched_at: Some(cached_at),
            stale: true,
        },
        now(),
    );
    let state = model.provider(CloudProvider::Aws);
    assert!(state.is_running(), "a run follows a stale cache");
    assert_eq!(state.clusters.len(), 1, "the cache shows at once");
    assert_eq!(state.fetched_at, Some(cached_at));

    // A fresh cache ends without running the CLI: its age stays.
    let mut fresh = CloudModel::new();
    for event in [
        CloudDiscoveryEvent::Cached {
            provider: CloudProvider::Aws,
            detection: Some(ready(CloudProvider::Aws)),
            clusters: vec![eks(&scope, "web")],
            fetched_at: Some(cached_at),
            stale: false,
        },
        CloudDiscoveryEvent::Finished {
            provider: CloudProvider::Aws,
            clusters: vec![eks(&scope, "web")],
            failed_scopes: 0,
        },
    ] {
        fresh.apply(&event, now());
    }
    assert_eq!(
        fresh.provider(CloudProvider::Aws).fetched_at,
        Some(cached_at)
    );
}

#[test]
fn hints_make_a_provider_degraded_and_a_failed_run_an_error_with_a_fix() {
    let mut model = CloudModel::new();
    let hint = oxikube_domain::cloud::SetupHint::with_command(
        "the GKE auth plugin is not installed",
        "gcloud components install gke-gcloud-auth-plugin",
    );
    model.apply(
        &CloudDiscoveryEvent::Detected(degraded(CloudProvider::Gcp, vec![hint])),
        now(),
    );
    assert_eq!(model.provider(CloudProvider::Gcp).phase(), Phase::Degraded);

    model.apply(
        &CloudDiscoveryEvent::Failed {
            provider: CloudProvider::Gcp,
            error: OxiError::auth("not logged in", false),
        },
        now(),
    );
    let state = model.provider(CloudProvider::Gcp);
    assert_eq!(state.phase(), Phase::Error);
    let fix = state.error.as_ref().expect("the failure has a fix");
    assert_eq!(fix.issue, Issue::NotLoggedIn);
    assert_eq!(fix.command.as_deref(), Some("gcloud auth login"));
}

#[test]
fn one_failing_scope_is_kept_apart_from_the_others() {
    let (prod, dev) = (
        aws_scope("prod", "eu-west-1"),
        aws_scope("dev", "eu-west-1"),
    );
    let mut model = CloudModel::new();
    model.apply(
        &CloudDiscoveryEvent::Detected(ready(CloudProvider::Aws)),
        now(),
    );
    model.apply(
        &CloudDiscoveryEvent::Started {
            provider: CloudProvider::Aws,
            scopes: vec![prod, dev.clone()],
        },
        now(),
    );
    model.apply(
        &CloudDiscoveryEvent::ScopeFailed {
            scope: dev.clone(),
            error: OxiError::auth("expired", false),
        },
        now(),
    );
    let failure = &model.provider(CloudProvider::Aws).failures[0];
    assert_eq!(failure.scope, dev);
    assert_eq!(
        failure.remediation.command.as_deref(),
        Some("aws sso login --profile dev"),
        "the command names the failing profile"
    );
}

#[test]
fn filters_narrow_the_list_and_reset_when_their_scope_goes() {
    let (prod, dev) = (
        aws_scope("prod", "eu-west-1"),
        aws_scope("dev", "us-east-1"),
    );
    let mut model = CloudModel::new();
    for event in run_events(
        CloudProvider::Aws,
        ready(CloudProvider::Aws),
        prod.clone(),
        vec![eks(&prod, "web"), eks(&dev, "api")],
    ) {
        model.apply(&event, now());
    }
    let state = model.provider(CloudProvider::Aws);
    assert_eq!(state.scopes(), [dev.clone(), prod.clone()]);
    assert_eq!(state.shown().count(), 2);

    model.set_filter(CloudProvider::Aws, Some(prod));
    let names: Vec<_> = model
        .provider(CloudProvider::Aws)
        .shown()
        .map(|c| c.name.clone())
        .collect();
    assert_eq!(names, ["web"]);

    model.apply(
        &CloudDiscoveryEvent::Finished {
            provider: CloudProvider::Aws,
            clusters: vec![eks(&dev, "api")],
            failed_scopes: 0,
        },
        now(),
    );
    assert!(model.provider(CloudProvider::Aws).filter.is_none());
}

#[test]
fn begin_is_refused_while_a_provider_runs_and_added_clusters_are_marked() {
    let scope: CloudScope = aws_scope("prod", "eu-west-1");
    let web = eks(&scope, "web");
    let mut model = CloudModel::new();
    assert!(model.begin(CloudProvider::Aws));
    assert!(
        !model.begin(CloudProvider::Aws),
        "a second click is a no-op"
    );
    assert!(model.end(CloudProvider::Aws));

    assert!(model.added_as(&web).is_none());
    model.set_added(CloudProvider::Aws, &[managed(&web)]);
    assert_eq!(
        model.added_as(&web).map(|c| c.to_string()),
        Some("eks-web".to_owned())
    );
    assert!(model.added_as(&eks(&scope, "api")).is_none());
}
