//! Refresh: the TTL cache, stale-while-revalidate, the scope fan-out, cancellation and hidden
//! providers.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures::StreamExt as _;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{
    CloudDetection, CloudDiscoveryPort as _, CloudProvider, CloudToolStatus, SetupHint,
};
use oxikube_testkit::{CloudCall, FakeCloudDiscoveryPort};

use super::scoped::{ScopedPort, shared};
use super::{Fixture, aws, aws_scope, collect, eks, finished, names};
use crate::cloud::{CloudDiscoveryEvent, DiscoveryConfig, RefreshMode};

#[tokio::test]
async fn a_first_refresh_lists_every_scope_and_caches_it() {
    let (prod, dev) = (
        aws_scope("prod", "eu-west-1"),
        aws_scope("dev", "us-east-1"),
    );
    let port = aws(
        [prod.clone(), dev.clone()],
        [eks(&prod, "web"), eks(&dev, "api")],
    );
    let f = Fixture::new(vec![port.clone()]);

    let events = f.refresh(CloudProvider::Aws, RefreshMode::IfStale).await;
    assert_eq!(
        names(&events),
        [
            "detected",
            "started",
            "scope-done",
            "scope-done",
            "finished"
        ]
    );
    let (clusters, failed) = finished(&events);
    assert_eq!(failed, 0);
    assert_eq!(
        clusters,
        [eks(&dev, "api"), eks(&prod, "web")],
        "sorted by key"
    );
    assert_eq!(f.service.cached(CloudProvider::Aws).await, clusters);
    assert!(!f.service.is_running(CloudProvider::Aws));
}

#[tokio::test]
async fn a_fresh_cache_answers_without_calling_the_port() {
    let prod = aws_scope("prod", "eu-west-1");
    let port = aws([prod.clone()], [eks(&prod, "web")]);
    let f = Fixture::new(vec![port.clone()]);
    f.refresh(CloudProvider::Aws, RefreshMode::IfStale).await;
    let calls = port.recorded_calls().len();

    f.advance(DiscoveryConfig::DEFAULT_TTL - Duration::from_secs(1));
    let events = f.refresh(CloudProvider::Aws, RefreshMode::IfStale).await;
    assert_eq!(names(&events), ["cached-fresh", "finished"]);
    assert_eq!(port.recorded_calls().len(), calls, "no CLI ran");
    let CloudDiscoveryEvent::Cached {
        clusters,
        detection,
        ..
    } = &events[0]
    else {
        panic!("cached first");
    };
    assert_eq!(*clusters, [eks(&prod, "web")]);
    assert_eq!(
        detection.as_ref().map(|d| d.status),
        Some(CloudToolStatus::Ready)
    );
    assert_eq!(finished(&events).0, [eks(&prod, "web")]);
}

#[tokio::test]
async fn a_stale_cache_is_sent_first_and_refreshed_behind_it() {
    let prod = aws_scope("prod", "eu-west-1");
    let port = aws([prod.clone()], [eks(&prod, "web")]);
    let f = Fixture::new(vec![port.clone()]);
    f.refresh(CloudProvider::Aws, RefreshMode::IfStale).await;

    f.advance(DiscoveryConfig::DEFAULT_TTL + Duration::from_secs(1));
    port.script()
        .list_clusters
        .push_ok(vec![eks(&prod, "web"), eks(&prod, "batch")]);
    let events = f.refresh(CloudProvider::Aws, RefreshMode::IfStale).await;
    assert_eq!(
        names(&events),
        [
            "cached-stale",
            "detected",
            "started",
            "scope-done",
            "finished"
        ]
    );
    let CloudDiscoveryEvent::Cached { clusters, .. } = &events[0] else {
        panic!("cached first");
    };
    assert_eq!(*clusters, [eks(&prod, "web")], "the old list, at once");
    let fresh = [eks(&prod, "batch"), eks(&prod, "web")];
    assert_eq!(finished(&events).0, fresh);
    assert_eq!(f.service.cached(CloudProvider::Aws).await, fresh);
}

#[tokio::test]
async fn force_runs_the_cli_even_when_the_cache_is_fresh() {
    let prod = aws_scope("prod", "eu-west-1");
    let port = aws([prod.clone()], [eks(&prod, "web")]);
    let f = Fixture::new(vec![port.clone()]);
    f.refresh(CloudProvider::Aws, RefreshMode::IfStale).await;
    let calls = port.recorded_calls().len();

    let events = f.refresh(CloudProvider::Aws, RefreshMode::Force).await;
    assert_eq!(names(&events)[0], "cached-stale");
    assert_eq!(names(&events).last(), Some(&"finished"));
    assert_eq!(port.recorded_calls().len(), calls * 2);
}

#[tokio::test]
async fn one_failing_scope_keeps_the_others_and_its_cached_clusters() {
    let (prod, dev) = (
        aws_scope("prod", "eu-west-1"),
        aws_scope("dev", "us-east-1"),
    );
    let fake = FakeCloudDiscoveryPort::ready(CloudProvider::Aws)
        .with_scopes([prod.clone(), dev.clone()])
        .with_clusters([eks(&prod, "web"), eks(&dev, "api")]);
    let f = Fixture::new(vec![Arc::new(fake)]);
    f.refresh(CloudProvider::Aws, RefreshMode::IfStale).await;

    // A second service over the same cache, whose `dev` profile's login expired.
    let fake = FakeCloudDiscoveryPort::ready(CloudProvider::Aws)
        .with_scopes([prod.clone(), dev.clone()])
        .with_clusters([eks(&prod, "web"), eks(&prod, "batch")]);
    let (_, providers) = shared(ScopedPort::new(fake).failing(dev.clone()));
    let g = Fixture::new(providers);
    copy_cache(&f, &g).await;
    let events = g.refresh(CloudProvider::Aws, RefreshMode::Force).await;

    let failed: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            CloudDiscoveryEvent::ScopeFailed { scope, error } => {
                Some((scope.clone(), error.kind()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(failed, [(dev.clone(), ErrorKind::Auth)]);
    let (clusters, failed_scopes) = finished(&events);
    assert_eq!(failed_scopes, 1);
    assert_eq!(
        clusters,
        [eks(&dev, "api"), eks(&prod, "batch"), eks(&prod, "web")],
        "prod is fresh, dev keeps its last listing"
    );
    assert_eq!(g.service.cached(CloudProvider::Aws).await, clusters);
}

#[tokio::test]
async fn cancel_stops_the_scopes_in_flight() {
    let (prod, dev) = (
        aws_scope("prod", "eu-west-1"),
        aws_scope("dev", "us-east-1"),
    );
    let fake =
        FakeCloudDiscoveryPort::ready(CloudProvider::Aws).with_scopes([prod.clone(), dev.clone()]);
    let (port, providers) = shared(ScopedPort::new(fake).hanging(prod).hanging(dev));
    let mut started = port.started();
    let f = Fixture::new(providers);

    let events = f
        .service
        .refresh(CloudProvider::Aws, None, RefreshMode::IfStale);
    for _ in 0..2 {
        started.next().await.expect("a listing starts");
    }
    assert!(f.service.is_running(CloudProvider::Aws));
    assert!(f.service.cancel(CloudProvider::Aws));
    assert!(
        !f.service.cancel(CloudProvider::Aws),
        "nothing left to cancel"
    );

    let events = collect(events).await;
    assert_eq!(names(&events), ["detected", "started", "cancelled"]);
    wait_for(|| port.dropped.load(Ordering::SeqCst) == 2).await;
    assert!(!f.service.is_running(CloudProvider::Aws));
    assert!(f.service.cached(CloudProvider::Aws).await.is_empty());
}

#[tokio::test]
async fn a_new_refresh_cancels_the_running_one() {
    let prod = aws_scope("prod", "eu-west-1");
    let fake = FakeCloudDiscoveryPort::ready(CloudProvider::Aws).with_scopes([prod.clone()]);
    let (port, providers) = shared(ScopedPort::new(fake).hanging(prod.clone()));
    let mut started = port.started();
    let f = Fixture::new(providers);

    let first = f
        .service
        .refresh(CloudProvider::Aws, None, RefreshMode::IfStale);
    started.next().await.expect("the first listing starts");
    let second = f
        .service
        .refresh(CloudProvider::Aws, None, RefreshMode::IfStale);
    assert_eq!(names(&collect(first).await).last(), Some(&"cancelled"));
    started.next().await.expect("the second listing starts");
    assert!(f.service.is_running(CloudProvider::Aws));
    f.service.cancel(CloudProvider::Aws);
    assert_eq!(names(&collect(second).await).last(), Some(&"cancelled"));
}

#[tokio::test]
async fn a_missing_cli_hides_the_provider_without_listing() {
    let port = Arc::new(FakeCloudDiscoveryPort::new(CloudProvider::Gcp));
    let f = Fixture::new(vec![port.clone()]);
    let events = f.refresh(CloudProvider::Gcp, RefreshMode::IfStale).await;
    assert_eq!(names(&events), ["detected", "finished"]);
    let CloudDiscoveryEvent::Detected(detection) = &events[0] else {
        panic!("detected first");
    };
    assert_eq!(detection.status, CloudToolStatus::NotInstalled);
    assert_eq!(port.recorded_calls(), [CloudCall::Detect]);

    // Within the TTL the cached detection answers: the CLI is not looked for again.
    let events = f.refresh(CloudProvider::Gcp, RefreshMode::IfStale).await;
    assert_eq!(names(&events), ["cached-fresh", "finished"]);
    assert_eq!(port.recorded_calls().len(), 1);
}

#[tokio::test]
async fn a_provider_without_an_adapter_is_not_installed() {
    let f = Fixture::new(Vec::new());
    let events = f.refresh(CloudProvider::Azure, RefreshMode::Force).await;
    assert_eq!(names(&events), ["detected", "finished"]);
    assert!(f.service.providers().is_empty());
}

#[tokio::test]
async fn a_missing_auth_plugin_keeps_the_provider_and_its_hint() {
    let project = oxikube_ports::CloudScope::Gcp {
        project: "shop".into(),
    };
    let mut detection = CloudDetection {
        hints: vec![SetupHint::with_command(
            "gke-gcloud-auth-plugin is not installed",
            "gcloud components install gke-gcloud-auth-plugin",
        )],
        ..block(FakeCloudDiscoveryPort::ready(CloudProvider::Gcp).detect())
    };
    detection.status = CloudToolStatus::Ready;
    let port = Arc::new(
        FakeCloudDiscoveryPort::ready(CloudProvider::Gcp)
            .with_detection(detection.clone())
            .with_scopes([project]),
    );
    let f = Fixture::new(vec![port]);
    let events = f.refresh(CloudProvider::Gcp, RefreshMode::IfStale).await;
    assert_eq!(
        names(&events),
        ["detected", "started", "scope-done", "finished"]
    );
    let CloudDiscoveryEvent::Detected(seen) = &events[0] else {
        panic!("detected first");
    };
    assert_eq!(*seen, detection);
}

#[tokio::test]
async fn a_scope_listing_failure_fails_the_run() {
    let port = aws([], []);
    port.script()
        .scopes
        .push_err(OxiError::auth("run `aws sso login`", false));
    let f = Fixture::new(vec![port]);
    let events = f.refresh(CloudProvider::Aws, RefreshMode::IfStale).await;
    assert_eq!(names(&events), ["detected", "failed"]);
    let Some(CloudDiscoveryEvent::Failed { error, .. }) = events.last() else {
        panic!("failed last");
    };
    assert_eq!(error.kind(), ErrorKind::Auth);
}

#[tokio::test]
async fn scopes_the_cli_no_longer_reports_leave_the_cache() {
    let (prod, dev) = (
        aws_scope("prod", "eu-west-1"),
        aws_scope("dev", "us-east-1"),
    );
    let port = aws(
        [prod.clone(), dev.clone()],
        [eks(&prod, "web"), eks(&dev, "api")],
    );
    let f = Fixture::new(vec![port.clone()]);
    f.refresh(CloudProvider::Aws, RefreshMode::IfStale).await;

    port.script().scopes.push_ok(vec![prod.clone()]);
    f.refresh(CloudProvider::Aws, RefreshMode::Force).await;
    assert_eq!(
        f.service.cached(CloudProvider::Aws).await,
        [eks(&prod, "web")]
    );
}

#[tokio::test]
async fn an_explicit_scope_list_narrows_the_run_and_the_freshness_check() {
    let (prod, dev) = (
        aws_scope("prod", "eu-west-1"),
        aws_scope("dev", "us-east-1"),
    );
    let port = aws(
        [prod.clone(), dev.clone()],
        [eks(&prod, "web"), eks(&dev, "api")],
    );
    let f = Fixture::new(vec![port.clone()]);
    let only_prod = Some(vec![prod.clone()]);
    let events = collect(f.service.refresh(
        CloudProvider::Aws,
        only_prod.clone(),
        RefreshMode::IfStale,
    ))
    .await;
    assert_eq!(finished(&events).0, [eks(&prod, "web")]);
    assert!(!port.recorded_calls().contains(&CloudCall::Scopes));

    // `prod` is fresh now; `dev` was never listed, so asking for it runs the CLI.
    let events = collect(
        f.service
            .refresh(CloudProvider::Aws, only_prod, RefreshMode::IfStale),
    )
    .await;
    assert_eq!(names(&events), ["cached-fresh", "finished"]);
    let events = collect(f.service.refresh(
        CloudProvider::Aws,
        Some(vec![dev.clone()]),
        RefreshMode::IfStale,
    ))
    .await;
    assert_eq!(names(&events)[0], "cached-stale");
    assert_eq!(finished(&events).0, [eks(&dev, "api")]);
}

/// Copies every cached row of `from` into `to`, as a restart over the same db would see them.
async fn copy_cache(from: &Fixture, to: &Fixture) {
    use oxikube_ports::{StatePort as _, StateTable};
    for table in ["cloud_scopes", "cloud_providers"] {
        let table = StateTable::new(table).unwrap();
        for (key, value) in from.state.table_list(&table, None).await.unwrap() {
            to.state.table_put(&table, &key, value).await.unwrap();
        }
    }
}

/// Polls `done` until it holds, failing after a few seconds.
async fn wait_for(done: impl Fn() -> bool) {
    for _ in 0..500 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the condition never held");
}

fn block<T>(future: impl std::future::Future<Output = oxikube_domain::OxiResult<T>>) -> T {
    futures::executor::block_on(future).unwrap()
}
