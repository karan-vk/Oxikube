//! kind integration for the health module: liveness and RBAC reviews against a real
//! apiserver. Skips cleanly when `OXIKUBE_TEST_CONTEXT` is unset.
#![cfg(feature = "integration")]

mod common;

use std::sync::Arc;

use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Client, Config};
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{ContextName, Gvr};
use oxikube_domain::session::ClusterSessionState;
use oxikube_kube::auth::CredentialRefresh;
use oxikube_kube::health::{
    AccessQuery, HealthEvent, Liveness, RBAC_DERIVED, RulesCache, can_i, capabilities_for_context,
    pooled_probe, probe_apiserver_version, probe_capabilities,
};
use oxikube_kube::pool::{ClientPool, PoolConfig};
use oxikube_testkit::integration::{ensure_kind_context, test_context};

use common::DEADLINE;
use common::health::{collect_events, fast_liveness, ready_session, session_states, shape};

/// A bearer token the apiserver does not know.
const INVALID_TOKEN: &str = "oxikube-invalid-token";

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
    auth.token = Some(INVALID_TOKEN.to_owned().into());
    config
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
    let (live, mut rx) = Liveness::spawn(fast_liveness(3), move || {
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

/// E03-S09: a kubeconfig context whose user presents an invalid token, connected through
/// the pool and watched by the liveness loop, takes its session from `Ready` to
/// `Degraded` on the first failed probe and to `Error` right after, because an inline
/// token is static and a rebuild cannot fix it.
#[tokio::test]
async fn bad_token_degrades_then_errors() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let context = ContextName::from("oxi-bad-token");
    let kubeconfig = kind.with_token_context(context.as_str(), INVALID_TOKEN);
    let user = kubeconfig
        .auth_infos
        .last()
        .and_then(|u| u.auth_info.as_ref());
    let refresh = CredentialRefresh::of(user.expect("bad-token user"));
    assert_eq!(refresh, CredentialRefresh::Static);
    let pool = Arc::new(kind.pool(kubeconfig));
    // Building the client does not contact the server: the connect itself succeeds.
    pool.get(&context).await.expect("client builds");

    let (_live, rx) = Liveness::spawn(fast_liveness(3), pooled_probe(pool, context, refresh));
    let events = collect_events(rx).await;
    assert_eq!(shape(&events), ["unhealthy", "failed"], "{events:?}");
    let error = events[1].error().expect("error");
    assert_eq!(error.kind(), ErrorKind::Auth);
    assert!(!error.is_retryable());

    let states = session_states(ready_session(), &events);
    assert_eq!(states[0], ClusterSessionState::Degraded);
    let reason = match &states[1] {
        ClusterSessionState::Error { reason } => reason,
        other => panic!("expected Error, got {other:?}"),
    };
    assert!(!reason.is_empty());
    for text in [reason.clone(), format!("{error} {error:?}")] {
        assert!(!text.contains(INVALID_TOKEN), "{text}");
    }
}

#[tokio::test]
async fn invalid_token_is_degraded_then_error_after_repeated_failures_when_a_refresh_could_help() {
    let Some(ctx) = test_context() else { return };
    let client = Client::try_from(bad_token_config(&ctx).await).expect("client");

    // `Unknown`: the 401 looks retryable, so the policy needs the failure threshold.
    let (_live, rx) = Liveness::spawn(fast_liveness(3), move || {
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
        fast_liveness(3),
        pooled_probe(pool.clone(), context, CredentialRefresh::Static),
    );
    let event = tokio::time::timeout(DEADLINE, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(event, HealthEvent::Healthy { .. }), "{event:?}");
    live.stop();
}

/// The kubeconfig with the CA removed from `context`'s cluster and TLS verification left
/// on, so the client falls back to the system trust store, which does not know kind's
/// self-signed CA: the handshake rejects the apiserver certificate. Also returns the
/// cluster's server URL.
fn kubeconfig_without_ca(context: &str) -> (Kubeconfig, String) {
    ensure_kind_context(context).expect("kind context");
    let mut kubeconfig = Kubeconfig::read().expect("read kubeconfig");
    let cluster_name = kubeconfig
        .contexts
        .iter()
        .find(|c| c.name == context)
        .and_then(|c| c.context.as_ref())
        .map(|c| c.cluster.clone())
        .expect("context has a cluster");
    let cluster = kubeconfig
        .clusters
        .iter_mut()
        .find(|c| c.name == cluster_name)
        .and_then(|c| c.cluster.as_mut())
        .expect("cluster entry");
    cluster.certificate_authority = None;
    cluster.certificate_authority_data = None;
    cluster.insecure_skip_tls_verify = Some(false);
    let server = cluster.server.clone().expect("cluster server");
    (kubeconfig, server)
}

#[tokio::test]
async fn an_untrusted_server_certificate_fails_on_the_first_probe() {
    let Some(ctx) = test_context() else { return };
    let (kubeconfig, server) = kubeconfig_without_ca(&ctx);
    let pool = Arc::new(ClientPool::new(kubeconfig, PoolConfig::default()));
    let context = ContextName::new(ctx.as_str());

    // A threshold of 3 would allow two more probes if the error were transient.
    let (_live, rx) = Liveness::spawn(
        fast_liveness(3),
        pooled_probe(pool, context, CredentialRefresh::Static),
    );
    let events = collect_events(rx).await;
    assert_eq!(shape(&events), ["unhealthy", "failed"], "{events:?}");
    let error = events[1].error().expect("error");
    assert_eq!(error.kind(), ErrorKind::Network, "{error:?}");
    assert!(!error.is_retryable(), "{error:?}");
    let text = format!("{error} {error:?}");
    assert!(
        text.contains("certificate was rejected (UnknownIssuer)"),
        "{text}"
    );
    // The message names the reason only, never the server.
    let host = server.trim_start_matches("https://");
    assert!(!text.contains(host), "{text}");
}
