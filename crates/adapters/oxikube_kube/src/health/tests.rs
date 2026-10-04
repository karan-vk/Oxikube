//! The probes and reviews over a fake HTTP layer: a real `kube::Client` talking to
//! [`FakeApi`], so `/version` parsing, `Status` bodies through `classify_with`, the review
//! POSTs and the pool's rebuild-once path run without a cluster. The review answers are
//! recorded from a kind cluster (`tests/fixtures/health/`).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use http::Method;
use kube::Client;
use kube::config::Kubeconfig;

use oxikube_domain::ids::{ContextName, Gvr};
use oxikube_domain::{Capability, ErrorKind, OxiError};
use serde_json::Value;
use tokio::sync::mpsc;

use super::*;
use crate::auth::CredentialRefresh;
use crate::fake_api::{FakeApi, status_body, version_body};
use crate::pool::{ClientFactory, ClientPool, ContextDefinition, PoolConfig, SystemClock};

const RULES_REVIEW: &str = include_str!("../../tests/fixtures/health/selfsubjectrulesreview.json");
const ACCESS_REVIEW: &str =
    include_str!("../../tests/fixtures/health/selfsubjectaccessreview.json");
const RULES_PATH: &str = "/apis/authorization.k8s.io/v1/selfsubjectrulesreviews";
const ACCESS_PATH: &str = "/apis/authorization.k8s.io/v1/selfsubjectaccessreviews";

fn config() -> LivenessConfig {
    LivenessConfig {
        interval: Duration::from_secs(30),
        failure_threshold: 3,
        ..Default::default()
    }
}

fn unauthorized() -> Value {
    status_body(401, "Unauthorized", "Unauthorized")
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).expect("fixture is JSON")
}

/// The liveness loop over `probe_apiserver_version` against `server`.
fn spawn_version_loop(
    server: &FakeApi,
    refresh: CredentialRefresh,
) -> (Liveness, mpsc::Receiver<HealthEvent>) {
    let client = server.client();
    Liveness::spawn(config(), move || {
        let client = client.clone();
        async move { probe_apiserver_version(&client, refresh).await }
    })
}

async fn next(rx: &mut mpsc::Receiver<HealthEvent>) -> HealthEvent {
    rx.recv().await.expect("event")
}

fn failures(event: &HealthEvent) -> Option<u32> {
    match event {
        HealthEvent::Unhealthy {
            consecutive_failures,
            ..
        } => Some(*consecutive_failures),
        _ => None,
    }
}

#[tokio::test(start_paused = true)]
async fn version_ok_is_healthy_with_the_git_version() {
    let server = FakeApi::new();
    server.reply("/version", 200, version_body("v1.35.0"));
    let (_live, mut rx) = spawn_version_loop(&server, CredentialRefresh::Unknown);
    match next(&mut rx).await {
        HealthEvent::Healthy { server_version } => assert_eq!(server_version, "v1.35.0"),
        other => panic!("{other:?}"),
    }
    let requests = server.requests();
    assert_eq!(requests[0].method, Method::GET);
    assert_eq!(requests[0].path, "/version");
}

#[tokio::test(start_paused = true)]
async fn unauthorized_with_unknown_refresh_degrades_then_fails_at_the_threshold() {
    let server = FakeApi::new();
    server.reply("/version", 401, unauthorized());
    let (_live, mut rx) = spawn_version_loop(&server, CredentialRefresh::Unknown);
    let first = next(&mut rx).await;
    assert_eq!(failures(&first), Some(1), "{first:?}");
    let error = first.error().expect("error");
    assert_eq!(error.kind(), ErrorKind::Auth);
    assert!(error.is_retryable(), "an unknown credential may refresh");
    assert_eq!(failures(&next(&mut rx).await), Some(2));
    let failed = next(&mut rx).await;
    assert!(
        matches!(&failed, HealthEvent::Failed { error } if error.kind() == ErrorKind::Auth),
        "{failed:?}"
    );
    assert!(rx.recv().await.is_none());
    assert_eq!(server.hits("/version"), 3);
}

#[tokio::test(start_paused = true)]
async fn unauthorized_with_a_static_credential_fails_at_once() {
    let server = FakeApi::new();
    server.reply("/version", 401, unauthorized());
    let (_live, mut rx) = spawn_version_loop(&server, CredentialRefresh::Static);
    assert_eq!(failures(&next(&mut rx).await), Some(1));
    let failed = next(&mut rx).await;
    assert!(
        matches!(&failed, HealthEvent::Failed { error }
            if error.kind() == ErrorKind::Auth && !error.is_retryable()),
        "{failed:?}"
    );
    assert!(rx.recv().await.is_none());
    assert_eq!(server.hits("/version"), 1);
}

#[tokio::test(start_paused = true)]
async fn unauthorized_then_ok_recovers_to_healthy() {
    let server = FakeApi::new();
    server
        .reply("/version", 401, unauthorized())
        .reply("/version", 200, version_body("v1.35.0"));
    let (_live, mut rx) = spawn_version_loop(&server, CredentialRefresh::Unknown);
    assert_eq!(failures(&next(&mut rx).await), Some(1));
    assert!(matches!(next(&mut rx).await, HealthEvent::Healthy { .. }));
}

#[tokio::test]
async fn fetch_rules_posts_a_review_and_reads_the_recorded_answer() {
    let server = FakeApi::new();
    server.reply(RULES_PATH, 201, json(RULES_REVIEW));
    let snapshot = fetch_rules(&server.client(), "default", CredentialRefresh::Unknown)
        .await
        .expect("rules");
    assert!(!snapshot.incomplete);
    assert_eq!(snapshot.evaluation_error, None);
    assert_eq!(snapshot.rules.len(), 5);
    let core = &snapshot.rules[0];
    assert_eq!(core.api_groups, [""]);
    assert_eq!(core.verbs, ["list", "watch"]);
    assert!(core.resources.iter().any(|r| r == "pods"));

    let report = capabilities_from_rules(&snapshot);
    for capability in [
        Capability::Mutate,
        Capability::Exec,
        Capability::Logs,
        Capability::PortForward,
    ] {
        assert_eq!(
            report.level(capability),
            AccessLevel::Denied,
            "{capability:?}"
        );
    }

    let request = &server.requests()[0];
    assert_eq!(request.method, Method::POST);
    let body = request.body.as_ref().expect("review body");
    assert_eq!(body["kind"], "SelfSubjectRulesReview");
    assert_eq!(body["spec"]["namespace"], "default");
}

#[tokio::test]
async fn a_rejected_rules_review_is_a_classified_auth_error() {
    let server = FakeApi::new();
    server.reply(RULES_PATH, 401, unauthorized());
    let err = fetch_rules(&server.client(), "default", CredentialRefresh::Static)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Auth);
    assert!(!err.is_retryable());
}

#[tokio::test]
async fn can_i_posts_the_attributes_and_reads_the_recorded_answer() {
    let server = FakeApi::new();
    server.reply(ACCESS_PATH, 201, json(ACCESS_REVIEW));
    let query = AccessQuery::new("create", Gvr::new("", "v1", "pods"))
        .subresource("exec")
        .namespace("default");
    let decision = can_i(&server.client(), &query, CredentialRefresh::Unknown)
        .await
        .expect("decision");
    assert_eq!(decision, AccessDecision::Denied { reason: None });

    let request = &server.requests()[0];
    assert_eq!(request.method, Method::POST);
    let body = request.body.as_ref().expect("review body");
    assert_eq!(body["kind"], "SelfSubjectAccessReview");
    let attrs = &body["spec"]["resourceAttributes"];
    assert_eq!(attrs["verb"], "create");
    assert_eq!(attrs["resource"], "pods");
    assert_eq!(attrs["subresource"], "exec");
    assert_eq!(attrs["namespace"], "default");
}

#[tokio::test]
async fn can_i_reads_an_allowed_answer() {
    let server = FakeApi::new();
    let mut answer = json(ACCESS_REVIEW);
    answer["status"]["allowed"] = Value::Bool(true);
    server.reply(ACCESS_PATH, 201, answer);
    let query = AccessQuery::new("list", Gvr::new("", "v1", "pods")).namespace("default");
    let decision = can_i(&server.client(), &query, CredentialRefresh::Unknown)
        .await
        .expect("decision");
    assert!(decision.is_allowed());
}

/// Hands out clients of one [`FakeApi`] and counts the builds.
struct FakeFactory {
    server: FakeApi,
    builds: AtomicUsize,
}

impl ClientFactory for FakeFactory {
    fn build(&self, _: &ContextDefinition, _: &PoolConfig) -> Result<Client, OxiError> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        Ok(self.server.client())
    }
}

const ONE_CONTEXT: &str = r"
apiVersion: v1
kind: Config
clusters:
- name: c
  cluster: {server: 'https://127.0.0.1:1'}
users:
- name: u
  user: {token: not-a-real-token}
contexts:
- name: a
  context: {cluster: c, user: u}
";

fn pool_over(server: &FakeApi) -> (Arc<ClientPool>, Arc<FakeFactory>) {
    let factory = Arc::new(FakeFactory {
        server: server.clone(),
        builds: AtomicUsize::new(0),
    });
    let kubeconfig = Kubeconfig::from_yaml(ONE_CONTEXT).expect("kubeconfig");
    let pool = ClientPool::with_parts(
        kubeconfig,
        PoolConfig::default(),
        factory.clone(),
        Arc::new(SystemClock),
    );
    (Arc::new(pool), factory)
}

// Real time: the pool builds on the blocking pool under a deadline.
#[tokio::test]
async fn pooled_probe_rebuilds_once_then_a_second_401_fails_at_once() {
    let server = FakeApi::new();
    server.reply("/version", 401, unauthorized());
    let (pool, factory) = pool_over(&server);
    let probe = pooled_probe(pool, ContextName::from("a"), CredentialRefresh::Unknown);
    let (_live, mut rx) = Liveness::spawn(config(), probe);
    assert_eq!(failures(&next(&mut rx).await), Some(1));
    let failed = next(&mut rx).await;
    assert!(
        matches!(&failed, HealthEvent::Failed { error }
            if error.kind() == ErrorKind::Auth && !error.is_retryable()),
        "{failed:?}"
    );
    assert_eq!(factory.builds.load(Ordering::SeqCst), 2, "one rebuild");
    assert_eq!(server.hits("/version"), 2, "one retry");
}

#[tokio::test]
async fn pooled_probe_recovers_when_the_rebuilt_client_is_accepted() {
    let server = FakeApi::new();
    server
        .reply("/version", 401, unauthorized())
        .reply("/version", 200, version_body("v1.35.0"));
    let (pool, factory) = pool_over(&server);
    let version = probe_context(&pool, &ContextName::from("a"), CredentialRefresh::Unknown)
        .await
        .expect("the retry succeeds");
    assert_eq!(version, "v1.35.0");
    assert_eq!(factory.builds.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn pooled_capabilities_read_the_review_through_the_pool() {
    let server = FakeApi::new();
    server.reply(RULES_PATH, 201, json(RULES_REVIEW));
    let (pool, _) = pool_over(&server);
    let cache = RulesCache::default();
    let context = ContextName::from("a");
    for _ in 0..2 {
        let report = capabilities_for_context(
            &pool,
            &cache,
            &context,
            "default",
            CredentialRefresh::Unknown,
        )
        .await
        .expect("report");
        assert_eq!(report.level(Capability::Exec), AccessLevel::Denied);
    }
    assert_eq!(
        server.hits(RULES_PATH),
        1,
        "the second answer comes from the cache"
    );
}
