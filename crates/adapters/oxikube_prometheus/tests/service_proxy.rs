//! Queries through the API server's service proxy: the same Prometheus answers as over a URL,
//! and the API server's own refusals as typed errors (E13-S05).

use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::ErrorKind;
use oxikube_domain::OxiError;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{
    PromEndpoint, PromqlEndpointPort, PromqlError, PromqlPort, ProxyMethod, ProxyResponse,
    ServicePort, ServiceProxyPort, ServiceRef,
};
use oxikube_prometheus::{PrometheusPort, ProxyPromqlClient, ServiceProxies};
use oxikube_testkit::{FakePromqlEndpointPort, FakeServiceProxyPort};

mod common;

fn endpoint() -> PromEndpoint {
    PromEndpoint::service(ServiceRef::new(
        "monitoring",
        "prometheus-operated",
        ServicePort::Name("web".into()),
    ))
    .with_path_prefix("/prometheus/")
}

fn client(proxy: &Arc<FakeServiceProxyPort>) -> (ProxyPromqlClient, Arc<FakePromqlEndpointPort>) {
    let direct = Arc::new(FakePromqlEndpointPort::new());
    let proxy: Arc<dyn ServiceProxyPort> = proxy.clone();
    (ProxyPromqlClient::new(proxy, direct.clone()), direct)
}

#[tokio::test]
async fn a_recorded_prometheus_answer_decodes_through_the_proxy() {
    let proxy = Arc::new(FakeServiceProxyPort::new());
    proxy.script().send.push_ok(FakeServiceProxyPort::json(
        200,
        &common::fixture("prometheus-3.5.0-instant-vector"),
    ));
    let (client, _) = client(&proxy);
    let series = client.instant_query(&endpoint(), "up", None).await.unwrap();
    assert!(!series.is_empty());
    let call = &proxy.recorded_calls()[0];
    assert_eq!(call.path, "/prometheus/api/v1/query");
    assert_eq!(call.params, [("query".to_owned(), "up".to_owned())]);
    assert_eq!(call.service.to_string(), endpoint().location());
}

#[tokio::test]
async fn range_queries_send_start_end_and_step() {
    let proxy = Arc::new(FakeServiceProxyPort::new());
    proxy.script().send.push_ok(FakeServiceProxyPort::json(
        200,
        &common::fixture("prometheus-3.5.0-range-matrix"),
    ));
    let (client, _) = client(&proxy);
    let series = client
        .range_query(&endpoint(), "rate(x[1m])", common::range())
        .await
        .unwrap();
    assert!(!series.is_empty());
    let call = &proxy.recorded_calls()[0];
    assert_eq!(call.path, "/prometheus/api/v1/query_range");
    let names: Vec<_> = call.params.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(names, ["query", "start", "end", "step"]);
    assert_eq!(call.params[3].1, "2");
}

#[tokio::test]
async fn a_url_endpoint_never_touches_the_proxy() {
    let proxy = Arc::new(FakeServiceProxyPort::new());
    let (client, direct) = client(&proxy);
    direct.script().push_ok(Vec::new());
    client
        .instant_query(&PromEndpoint::new("https://prom.example.com"), "up", None)
        .await
        .unwrap();
    assert!(proxy.recorded_calls().is_empty());
    assert_eq!(direct.recorded_calls().len(), 1);
}

#[tokio::test]
async fn the_api_servers_refusals_are_typed_and_say_whose_they_are() {
    let cases = [
        (403, "services/proxy is forbidden", "service proxy"),
        (401, "Unauthorized", "rejected the credentials"),
        (
            404,
            "services \"prometheus-operated\" not found",
            "does not exist",
        ),
        (
            503,
            "no endpoints available for service",
            "no ready endpoint",
        ),
    ];
    for (status, message, needle) in cases {
        let proxy = Arc::new(FakeServiceProxyPort::new());
        proxy
            .script()
            .send
            .push_ok(FakeServiceProxyPort::api_error(status, message));
        let (client, _) = client(&proxy);
        let error = client
            .instant_query(&endpoint(), "up", None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains(needle), "{status}: {error}");
        match status {
            403 => assert!(matches!(error, PromqlError::Forbidden(_))),
            401 => assert!(matches!(error, PromqlError::Unauthorized(_))),
            404 | 503 => assert!(error.is_absent(), "{error}"),
            _ => unreachable!(),
        }
    }
}

#[tokio::test]
async fn the_services_own_errors_are_decoded_like_over_a_url() {
    let proxy = Arc::new(FakeServiceProxyPort::new());
    proxy
        .script()
        .send
        .push_ok(FakeServiceProxyPort::json(
            400,
            &common::fixture("prometheus-3.5.0-error-bad-data"),
        ))
        .push_ok(ProxyResponse {
            status: 404,
            content_type: Some("text/plain".into()),
            body: b"404 page not found".to_vec(),
            truncated: false,
            api_server_message: None,
        })
        .push_ok(ProxyResponse {
            status: 200,
            content_type: Some("application/json".into()),
            body: Vec::new(),
            truncated: true,
            api_server_message: None,
        });
    let (client, _) = client(&proxy);
    let bad = client
        .instant_query(&endpoint(), "up", None)
        .await
        .unwrap_err();
    assert!(bad.is_query_error(), "{bad}");
    let wrong_prefix = client
        .instant_query(&endpoint(), "up", None)
        .await
        .unwrap_err();
    assert!(
        matches!(wrong_prefix, PromqlError::BadResponse(_)),
        "{wrong_prefix}"
    );
    assert!(wrong_prefix.to_string().contains("path prefix"));
    let big = client
        .instant_query(&endpoint(), "up", None)
        .await
        .unwrap_err();
    assert!(big.to_string().contains("byte limit"), "{big}");
}

#[tokio::test]
async fn transport_failures_map_onto_the_taxonomy() {
    let proxy = Arc::new(FakeServiceProxyPort::new());
    proxy
        .script()
        .send
        .push_err(OxiError::timeout("did not answer"))
        .push_err(OxiError::network("connection reset"))
        .push_err(OxiError::auth("token expired", false))
        .push_err(OxiError::validation("bad path"));
    let (client, _) = client(&proxy);
    let mut kinds = Vec::new();
    for _ in 0..4 {
        kinds.push(
            client
                .instant_query(&endpoint(), "up", None)
                .await
                .unwrap_err(),
        );
    }
    assert!(matches!(kinds[0], PromqlError::Timeout(_)));
    assert!(kinds[1].is_absent());
    assert!(matches!(kinds[2], PromqlError::Unauthorized(_)));
    assert!(matches!(kinds[3], PromqlError::InvalidEndpoint(_)));
}

#[tokio::test]
async fn a_very_long_query_is_a_form_post() {
    let proxy = Arc::new(FakeServiceProxyPort::new());
    proxy
        .script()
        .send
        .push_ok(FakeServiceProxyPort::prometheus_ok());
    let seen = Arc::new(parking_lot::Mutex::new(None));
    let recorder = seen.clone();
    proxy.respond_with(move |_, request| {
        *recorder.lock() = Some(request.method);
        None
    });
    let (client, _) = client(&proxy);
    let long = format!("sum({})", "x + ".repeat(1000));
    client
        .instant_query(&endpoint(), &long, None)
        .await
        .unwrap();
    // The first call used the queued answer; ask again to see the responder's view.
    let _ = client.instant_query(&endpoint(), &long, None).await;
    assert_eq!(*seen.lock(), Some(ProxyMethod::PostForm));
    let _ = client.instant_query(&endpoint(), "up", None).await;
    assert_eq!(*seen.lock(), Some(ProxyMethod::Get));
}

#[tokio::test]
async fn an_empty_query_and_a_bad_range_never_leave_the_machine() {
    let proxy = Arc::new(FakeServiceProxyPort::new());
    let (client, _) = client(&proxy);
    assert!(
        client
            .instant_query(&endpoint(), "  ", None)
            .await
            .unwrap_err()
            .is_query_error()
    );
    let mut range = common::range();
    range.step = Duration::ZERO;
    assert!(
        client
            .range_query(&endpoint(), "up", range)
            .await
            .unwrap_err()
            .is_query_error()
    );
    assert!(proxy.recorded_calls().is_empty());
}

struct Proxies(Option<Arc<FakeServiceProxyPort>>);

impl ServiceProxies for Proxies {
    fn service_proxy(&self, _: &ClusterId) -> Option<Arc<dyn ServiceProxyPort>> {
        self.0.clone().map(|p| p as Arc<dyn ServiceProxyPort>)
    }
}

fn cluster() -> ClusterId {
    ClusterId::new("/kubeconfig", &ContextName::new("kind-test"))
}

#[tokio::test]
async fn the_cluster_port_routes_a_service_endpoint_through_that_clusters_proxy() {
    let proxy = Arc::new(FakeServiceProxyPort::new());
    proxy
        .script()
        .send
        .push_ok(FakeServiceProxyPort::prometheus_ok());
    let port = PrometheusPort::new(Arc::new(FakePromqlEndpointPort::new()))
        .with_proxies(Arc::new(Proxies(Some(proxy.clone()))));
    port.set_endpoint(cluster(), endpoint());
    assert!(port.is_available(&cluster()).await.unwrap());
    assert_eq!(proxy.recorded_calls().len(), 1);
}

#[tokio::test]
async fn a_service_endpoint_on_a_disconnected_cluster_says_so() {
    let port = PrometheusPort::new(Arc::new(FakePromqlEndpointPort::new()))
        .with_proxies(Arc::new(Proxies(None)));
    port.set_endpoint(cluster(), endpoint());
    // Not connected is "not available", with the reason on a query.
    assert!(!port.is_available(&cluster()).await.unwrap());
    let error = port
        .query_instant(&cluster(), "up", None)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Network);
    assert!(
        error.message().contains("not connected"),
        "{}",
        error.message()
    );
}
