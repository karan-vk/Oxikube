//! kind integration for the health module: liveness and RBAC reviews against a real
//! apiserver. Skips cleanly when `OXIKUBE_TEST_CONTEXT` is unset.
#![cfg(feature = "integration")]

use std::time::Duration;

use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Client, Config};
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{ContextName, Gvr};
use oxikube_kube::auth::CredentialRefresh;
use oxikube_kube::health::{
    AccessQuery, HealthEvent, Liveness, LivenessConfig, RBAC_DERIVED, RulesCache, can_i,
    probe_apiserver_version, probe_capabilities,
};
use oxikube_testkit::integration::{ensure_kind_context, test_context};

/// Generous bound for "eventually" waits; the loops below finish in well under a second.
const DEADLINE: Duration = Duration::from_secs(60);

async fn kind_config(context: &str) -> Config {
    ensure_kind_context(context).expect("kind context");
    let options = KubeConfigOptions {
        context: Some(context.to_owned()),
        ..Default::default()
    };
    let kubeconfig = Kubeconfig::read().expect("read kubeconfig");
    Config::from_custom_kubeconfig(kubeconfig, &options)
        .await
        .expect("kubeconfig for context")
}

/// The same cluster, but the user presents only a token the server does not know. The
/// client certificate is removed: kind users authenticate with one, and a valid
/// certificate would win over the bad token.
async fn bad_token_config(context: &str) -> Config {
    let mut config = kind_config(context).await;
    let auth = &mut config.auth_info;
    auth.client_certificate = None;
    auth.client_certificate_data = None;
    auth.client_key = None;
    auth.client_key_data = None;
    auth.exec = None;
    auth.token_file = None;
    auth.token = Some("oxikube-invalid-token".to_owned().into());
    config
}

fn fast_config(threshold: u32) -> LivenessConfig {
    LivenessConfig {
        interval: Duration::from_millis(200),
        probe_timeout: Duration::from_secs(10),
        failure_threshold: threshold,
        backoff: backon::ExponentialBuilder::new()
            .with_min_delay(Duration::from_millis(50))
            .with_max_delay(Duration::from_millis(200))
            .without_max_times(),
    }
}

/// Drains events until the loop ends, within [`DEADLINE`].
async fn collect_events(mut rx: tokio::sync::mpsc::Receiver<HealthEvent>) -> Vec<HealthEvent> {
    tokio::time::timeout(DEADLINE, async {
        let mut events = vec![];
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        events
    })
    .await
    .expect("liveness loop did not finish within the deadline")
}

fn shape(events: &[HealthEvent]) -> Vec<&'static str> {
    events
        .iter()
        .map(|e| match e {
            HealthEvent::Healthy { .. } => "healthy",
            HealthEvent::Unhealthy { .. } => "unhealthy",
            HealthEvent::Failed { .. } => "failed",
        })
        .collect()
}

#[tokio::test]
async fn valid_credentials_are_healthy_and_admin_has_every_rbac_capability() {
    let Some(ctx) = test_context() else { return };
    let client = Client::try_from(kind_config(&ctx).await).expect("client");

    let version = probe_apiserver_version(&client, CredentialRefresh::Static)
        .await
        .expect("version");
    assert!(version.starts_with('v'), "{version}");

    let cache = RulesCache::default();
    let report = probe_capabilities(
        &client,
        &cache,
        &ContextName::new(ctx.as_str()),
        "default",
        CredentialRefresh::Static,
    )
    .await
    .expect("rules review");
    assert_eq!(report.granted, RBAC_DERIVED, "{report:?}");
    assert!(report.denied().is_empty());

    let query = AccessQuery::new("create", Gvr::new("", "v1", "pods"))
        .subresource("exec")
        .namespace("default");
    let decision = can_i(&client, &query, CredentialRefresh::Static)
        .await
        .expect("access review");
    assert!(decision.is_allowed(), "{decision:?}");

    // Liveness over the same client reports Healthy repeatedly.
    let probe_client = client.clone();
    let (live, mut rx) = Liveness::spawn(fast_config(3), move || {
        let client = probe_client.clone();
        async move { probe_apiserver_version(&client, CredentialRefresh::Static).await }
    });
    for _ in 0..2 {
        let event = tokio::time::timeout(DEADLINE, rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(event, HealthEvent::Healthy { .. }), "{event:?}");
    }
    live.stop();
}

#[tokio::test]
async fn invalid_token_is_degraded_then_error_immediately_when_the_credential_is_static() {
    let Some(ctx) = test_context() else { return };
    let config = bad_token_config(&ctx).await;
    let refresh = CredentialRefresh::of(&config.auth_info);
    assert_eq!(refresh, CredentialRefresh::Static);
    let client = Client::try_from(config).expect("client");

    let (_live, rx) = Liveness::spawn(fast_config(3), move || {
        let client = client.clone();
        async move { probe_apiserver_version(&client, refresh).await }
    });
    let events = collect_events(rx).await;
    assert_eq!(shape(&events), ["unhealthy", "failed"]);
    let error = events[1].error().expect("error");
    assert_eq!(error.kind(), ErrorKind::Auth);
    assert!(!error.is_retryable());
    assert!(!error.to_string().contains("oxikube-invalid-token"));
}

#[tokio::test]
async fn invalid_token_is_degraded_then_error_after_repeated_failures_when_a_refresh_could_help() {
    let Some(ctx) = test_context() else { return };
    let client = Client::try_from(bad_token_config(&ctx).await).expect("client");

    // `Unknown`: the 401 looks retryable, so the policy needs the failure threshold.
    let (_live, rx) = Liveness::spawn(fast_config(3), move || {
        let client = client.clone();
        async move { probe_apiserver_version(&client, CredentialRefresh::Unknown).await }
    });
    let events = collect_events(rx).await;
    assert_eq!(shape(&events), ["unhealthy", "unhealthy", "failed"]);
    assert_eq!(events[2].error().unwrap().kind(), ErrorKind::Auth);
}

#[tokio::test]
async fn rules_review_with_an_invalid_token_is_an_auth_error() {
    let Some(ctx) = test_context() else { return };
    let client = Client::try_from(bad_token_config(&ctx).await).expect("client");
    let err = probe_capabilities(
        &client,
        &RulesCache::default(),
        &ContextName::new(ctx.as_str()),
        "default",
        CredentialRefresh::Static,
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Auth, "{err:?}");
}

#[tokio::test]
async fn pooled_probe_and_capabilities_work_against_the_pool() {
    use std::sync::Arc;

    use oxikube_kube::health::{capabilities_for_context, pooled_probe};
    use oxikube_kube::pool::{ClientPool, PoolConfig};

    let Some(ctx) = test_context() else { return };
    ensure_kind_context(&ctx).expect("kind context");
    let pool = Arc::new(ClientPool::new(
        Kubeconfig::read().expect("read kubeconfig"),
        PoolConfig::default(),
    ));
    let context = ContextName::new(ctx.as_str());

    let report = capabilities_for_context(
        &pool,
        &RulesCache::default(),
        &context,
        "default",
        CredentialRefresh::Static,
    )
    .await
    .expect("capabilities through the pool");
    assert_eq!(report.granted, RBAC_DERIVED);

    let (live, mut rx) = Liveness::spawn(
        fast_config(3),
        pooled_probe(pool.clone(), context, CredentialRefresh::Static),
    );
    let event = tokio::time::timeout(DEADLINE, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(event, HealthEvent::Healthy { .. }), "{event:?}");
    live.stop();
}
