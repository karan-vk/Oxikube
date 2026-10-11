//! Kind integration for E13-S05: `ServiceProxyPort` against the real API server's service proxy.
//!
//! An nginx pod behind a Service in a namespace of the test's own answers as a Service does; the
//! API server's own refusals are real ones: a Service that does not exist (404), one without a
//! ready endpoint (503), an account that may not use `services/proxy` (403). The proxy returns them
//! as answers, never as errors, and tells the API server's `Status` apart from the Service's own
//! 404.
//!
//! Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use std::time::Duration;

use k8s_openapi::api::rbac::v1::PolicyRule;
use oxikube_kube::KubeServiceProxy;
use oxikube_ports::{
    ProxyMethod, ProxyRequest, ProxyResponse, ServicePort, ServiceProxyPort, ServiceRef,
};
use oxikube_testkit::integration::TestNamespace;

use common::portforward::{create_nginx_pod, create_nginx_service, wait_ready};
use common::{DEADLINE, TestServiceAccount, wait_until};

fn request(path: &str) -> ProxyRequest {
    ProxyRequest {
        timeout: Duration::from_secs(20),
        ..ProxyRequest::get(path)
    }
}

async fn send(
    proxy: &KubeServiceProxy,
    service: &ServiceRef,
    request: ProxyRequest,
) -> ProxyResponse {
    proxy
        .send(service, request)
        .await
        .expect("an answer from the API server")
}

#[tokio::test]
async fn the_proxy_reaches_a_service_and_tells_its_answers_from_the_api_servers() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_nginx_pod(&client, ns.name(), "web", "web").await;
    create_nginx_service(&client, ns.name(), "web", "web").await;
    // A Service nothing backs: no endpoint to forward to.
    create_nginx_service(&client, ns.name(), "orphan", "nobody").await;
    wait_ready(&client, ns.name(), "web").await;
    let proxy = KubeServiceProxy::new((*client).clone());

    // By port name and by number; the Service answers 200.
    for port in [ServicePort::Name("web".into()), ServicePort::Number(80)] {
        let service = ServiceRef::new(ns.name(), "web", port);
        let answer = wait_until("nginx to answer through the proxy", DEADLINE, || async {
            let answer = send(&proxy, &service, request("/")).await;
            (answer.status == 200).then_some(answer)
        })
        .await;
        assert!(String::from_utf8_lossy(&answer.body).contains("nginx"));
        assert_eq!(answer.api_server_message, None, "the Service's own answer");
        assert!(!answer.truncated);
    }

    let web = ServiceRef::new(ns.name(), "web", ServicePort::Name("web".into()));
    // Query parameters reach the Service (nginx ignores them and still answers).
    let mut with_params = request("/");
    with_params.params = vec![("query".into(), "sum(rate(x[5m])) by (pod)".into())];
    assert_eq!(send(&proxy, &web, with_params).await.status, 200);

    // A form POST is forwarded as a POST: nginx refuses it for a static file, which only the
    // Service can say.
    let mut post = request("/");
    post.method = ProxyMethod::PostForm;
    post.params = vec![("query".into(), "up".into())];
    let posted = send(&proxy, &web, post).await;
    assert_eq!(
        posted.status,
        405,
        "{}",
        String::from_utf8_lossy(&posted.body)
    );
    assert_eq!(posted.api_server_message, None);

    // The Service's own 404 is not the API server's.
    let missing_page = send(&proxy, &web, request("/no-such-page")).await;
    assert_eq!(missing_page.status, 404);
    assert_eq!(missing_page.api_server_message, None);

    // A Service that does not exist: the API server's 404 with its message.
    let nobody = ServiceRef::new(ns.name(), "no-such-service", ServicePort::Number(80));
    let gone = send(&proxy, &nobody, request("/")).await;
    assert_eq!(gone.status, 404);
    assert!(
        gone.api_server_message
            .as_deref()
            .is_some_and(|m| m.contains("no-such-service")),
        "{:?}",
        gone.api_server_message
    );

    // A Service with no ready endpoint: the API server answers 503, which the connection's client
    // retries until the request's deadline, so the caller gets a timeout that names the cause.
    let orphan = ServiceRef::new(ns.name(), "orphan", ServicePort::Name("web".into()));
    let mut short = request("/");
    short.timeout = Duration::from_secs(3);
    let error = proxy.send(&orphan, short).await.unwrap_err();
    assert_eq!(error.kind(), oxikube_domain::ErrorKind::Timeout);
    assert!(
        error.message().contains("no ready endpoint"),
        "{}",
        error.message()
    );

    // A body over the limit is dropped, not buffered.
    let mut tiny = request("/");
    tiny.max_response_bytes = 16;
    let big = send(&proxy, &web, tiny).await;
    assert!(big.truncated && big.body.is_empty(), "{big:?}");
}

#[tokio::test]
async fn an_account_that_may_not_use_the_proxy_gets_a_403_that_says_so() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let admin = kind.admin_client().await;
    // Rights on pods only: nothing on services or services/proxy.
    let pods_only = PolicyRule {
        api_groups: Some(vec![String::new()]),
        resources: Some(vec!["pods".into()]),
        verbs: vec!["get".into(), "list".into()],
        ..PolicyRule::default()
    };
    let account =
        TestServiceAccount::create(&admin, ns.name(), "oxi-no-proxy", vec![pods_only]).await;
    let context = oxikube_domain::ids::ContextName::from("oxi-no-proxy");
    let client = kind
        .pool(kind.with_token_context(context.as_str(), &account.token))
        .get(&context)
        .await
        .expect("restricted client");
    let proxy = KubeServiceProxy::new((*client).clone());
    let service = ServiceRef::new(ns.name(), "web", ServicePort::Name("web".into()));

    let denied = wait_until("RBAC to deny the proxy", DEADLINE, || async {
        let answer = send(&proxy, &service, request("/")).await;
        (answer.status == 403).then_some(answer)
    })
    .await;
    let message = denied
        .api_server_message
        .expect("the API server's own refusal");
    assert!(
        message.contains("services/proxy") || message.contains("forbidden"),
        "{message}"
    );
}

#[tokio::test]
async fn a_path_that_could_change_the_route_never_reaches_the_cluster() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let client = kind.admin_client().await;
    let proxy = KubeServiceProxy::new((*client).clone());
    let service = ServiceRef::new("default", "web", ServicePort::Number(80));
    let error = proxy
        .send(&service, request("/../../secrets"))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), oxikube_domain::ErrorKind::Validation);
}

#[tokio::test]
async fn detection_as_an_account_that_may_not_list_services_fails_as_forbidden_not_as_absent() {
    use oxikube_ports::{DetectionFailure, DetectionState, ProviderDetectorPort};
    use oxikube_prometheus::ProviderDetector;

    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let admin = kind.admin_client().await;
    // Rights on pods only: it may not list Services anywhere.
    let pods_only = PolicyRule {
        api_groups: Some(vec![String::new()]),
        resources: Some(vec!["pods".into()]),
        verbs: vec!["get".into(), "list".into()],
        ..PolicyRule::default()
    };
    let account =
        TestServiceAccount::create(&admin, ns.name(), "oxi-no-services", vec![pods_only]).await;
    let context = oxikube_domain::ids::ContextName::from("oxi-no-services");
    let client = kind
        .pool(kind.with_token_context(context.as_str(), &account.token))
        .get(&context)
        .await
        .expect("restricted client");
    let resources = std::sync::Arc::new(common::resources::adapter(&client));
    let proxy = std::sync::Arc::new(KubeServiceProxy::new((*client).clone()));
    let detector = ProviderDetector::new();

    let result = wait_until("RBAC to deny listing services", DEADLINE, || async {
        let result = detector.detect(resources.clone(), proxy.clone()).await;
        matches!(
            result.state,
            DetectionState::Failed(DetectionFailure::Forbidden(_))
        )
        .then_some(result)
    })
    .await;

    let DetectionState::Failed(failure) = &result.state else {
        unreachable!("checked above");
    };
    assert!(failure.is_forbidden());
    assert!(
        failure.reason().contains("configure the endpoint by hand"),
        "{}",
        failure.reason()
    );
    assert!(
        result
            .probes
            .iter()
            .all(|p| matches!(p.outcome, oxikube_ports::ProbeOutcome::Skipped(_))),
        "nothing could be checked: {:?}",
        result.probes
    );
}
