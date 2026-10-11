//! The per-cluster `PromqlPort` adapter on top of a scripted endpoint client.

use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{PromEndpoint, PromqlError, PromqlPort, PromqlSeries, TimeRange};
use oxikube_prometheus::PrometheusPort;
use oxikube_testkit::{FakePromqlEndpointPort, PromqlEndpointCall};

fn cluster() -> ClusterId {
    ClusterId::new("/kubeconfig", &ContextName::new("kind-test"))
}

fn port() -> (PrometheusPort, Arc<FakePromqlEndpointPort>) {
    let fake = Arc::new(FakePromqlEndpointPort::new());
    let port = PrometheusPort::new(fake.clone());
    port.set_endpoint(cluster(), PromEndpoint::new("http://prom.monitoring:9090"));
    (port, fake)
}

fn range() -> TimeRange {
    TimeRange {
        start: Timestamp::UNIX_EPOCH,
        end: Timestamp::UNIX_EPOCH,
        step: Duration::from_secs(1),
    }
}

#[tokio::test]
async fn a_cluster_without_an_endpoint_has_no_prometheus() {
    let port = PrometheusPort::new(Arc::new(FakePromqlEndpointPort::new()));
    assert!(!port.is_available(&cluster()).await.unwrap());
    let err = port
        .query_instant(&cluster(), "up", None)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
}

#[tokio::test]
async fn availability_distinguishes_absent_from_refused() {
    let (port, fake) = port();
    fake.script().push_ok(Vec::new());
    assert!(port.is_available(&cluster()).await.unwrap());

    fake.script()
        .push_err(PromqlError::Unreachable("refused".into()));
    assert!(
        !port.is_available(&cluster()).await.unwrap(),
        "unreachable is the normal 'not installed'"
    );

    fake.script()
        .push_err(PromqlError::Timeout(Duration::from_secs(1)));
    assert!(!port.is_available(&cluster()).await.unwrap());

    fake.script()
        .push_err(PromqlError::Unauthorized("401".into()));
    let err = port.is_available(&cluster()).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Auth);
    assert!(matches!(
        PromqlError::of(&err),
        Some(PromqlError::Unauthorized(_))
    ));
}

#[tokio::test]
async fn queries_pass_through_and_errors_keep_their_taxonomy() {
    let (port, fake) = port();
    let series = vec![PromqlSeries {
        labels: Default::default(),
        points: Vec::new(),
    }];
    fake.script().push_ok(series.clone());
    assert_eq!(
        port.query_instant(&cluster(), "up", None).await.unwrap(),
        series
    );

    fake.script().push_err(PromqlError::BadQuery {
        message: "parse error".into(),
    });
    let err = port
        .query_range(&cluster(), "up{", range())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(PromqlError::of(&err).unwrap().is_query_error());

    let calls = fake.recorded_calls();
    assert!(
        matches!(&calls[0], PromqlEndpointCall::Instant { base_url, query, .. } if base_url == "http://prom.monitoring:9090" && query == "up")
    );
    assert!(matches!(&calls[1], PromqlEndpointCall::Range { query, .. } if query == "up{"));
}

#[tokio::test]
async fn endpoints_are_per_cluster_and_replaceable() {
    let (port, fake) = port();
    let other = ClusterId::new("/kubeconfig", &ContextName::new("other"));
    assert!(!port.is_available(&other).await.unwrap());
    port.set_endpoint(other.clone(), PromEndpoint::new("http://other:9090"));
    fake.script().push_ok(Vec::new());
    port.query_instant(&other, "up", None).await.unwrap();
    port.clear_endpoint(&other);
    assert!(port.endpoint(&other).is_none());
    assert_eq!(
        port.query_instant(&other, "up", None)
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
}
