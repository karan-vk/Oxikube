//! Every failure maps to the expected [`PromqlError`], so the UI can tell "no Prometheus" from
//! "the query is wrong".

mod common;

use std::time::Duration;

use common::{endpoint, range};
use oxikube_ports::{PromEndpoint, PromqlEndpointPort, PromqlError};
use oxikube_prometheus::PromqlClient;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn answer(server: &MockServer, template: ResponseTemplate) {
    Mock::given(method("GET"))
        .respond_with(template)
        .mount(server)
        .await;
}

async fn instant(ep: &PromEndpoint) -> PromqlError {
    PromqlClient::new()
        .instant_query(ep, "up", None)
        .await
        .unwrap_err()
}

fn json(status: u16, body: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(body.as_bytes().to_vec(), "application/json")
}

#[tokio::test]
async fn http_statuses() {
    let server = MockServer::start().await;
    let ep = endpoint(&server);

    answer(
        &server,
        ResponseTemplate::new(401).set_body_string("Unauthorized"),
    )
    .await;
    assert!(matches!(instant(&ep).await, PromqlError::Unauthorized(_)));
    server.reset().await;

    answer(
        &server,
        json(
            403,
            r#"{"status":"error","errorType":"forbidden","error":"no access to tenant"}"#,
        ),
    )
    .await;
    assert!(matches!(instant(&ep).await, PromqlError::Forbidden(m) if m.contains("no access")));
    server.reset().await;

    answer(
        &server,
        json(
            400,
            r#"{"status":"error","errorType":"bad_data","error":"parse error: unexpected end"}"#,
        ),
    )
    .await;
    let err = instant(&ep).await;
    assert!(
        matches!(&err, PromqlError::BadQuery { message } if message.contains("parse error")),
        "{err:?}"
    );
    assert!(err.is_query_error() && !err.is_absent());
    server.reset().await;

    answer(
        &server,
        json(
            422,
            r#"{"status":"error","errorType":"execution","error":"duplicate series"}"#,
        ),
    )
    .await;
    assert!(matches!(instant(&ep).await, PromqlError::BadQuery { .. }));
    server.reset().await;

    for status in [500u16, 502, 503, 504] {
        answer(
            &server,
            ResponseTemplate::new(status).set_body_string("<html>upstream</html>"),
        )
        .await;
        let err = instant(&ep).await;
        assert!(
            matches!(&err, PromqlError::ServerError { status: s, .. } if *s == status),
            "{status}: {err:?}"
        );
        assert!(err.is_retryable());
        server.reset().await;
    }

    answer(&server, json(503, r#"{"status":"error","errorType":"timeout","error":"query timed out in expression evaluation"}"#)).await;
    assert!(
        matches!(instant(&ep).await, PromqlError::ServerError { status: 503, message } if message.contains("timed out"))
    );
    server.reset().await;

    answer(
        &server,
        ResponseTemplate::new(404).set_body_string("404 page not found"),
    )
    .await;
    assert!(matches!(instant(&ep).await, PromqlError::BadResponse(m) if m.contains("path prefix")));
}

#[tokio::test]
async fn a_200_that_is_not_prometheus_is_a_bad_response() {
    let server = MockServer::start().await;
    answer(
        &server,
        ResponseTemplate::new(200).set_body_raw(b"<html>Sign in</html>".to_vec(), "text/html"),
    )
    .await;
    assert!(matches!(
        instant(&endpoint(&server)).await,
        PromqlError::BadResponse(_)
    ));
}

#[tokio::test]
async fn an_error_envelope_on_a_200_is_still_an_error() {
    let server = MockServer::start().await;
    answer(
        &server,
        json(
            200,
            r#"{"status":"error","errorType":"bad_data","error":"nope"}"#,
        ),
    )
    .await;
    assert!(matches!(
        instant(&endpoint(&server)).await,
        PromqlError::BadQuery { .. }
    ));
}

#[tokio::test]
async fn connection_refused_is_unreachable() {
    // Bind then drop, so the port is closed.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let ep =
        PromEndpoint::new(format!("http://127.0.0.1:{port}")).with_timeout(Duration::from_secs(5));
    let err = instant(&ep).await;
    assert!(matches!(err, PromqlError::Unreachable(_)), "{err:?}");
    assert!(err.is_absent() && err.is_retryable());
    // The message is about the connection, and carries no query string.
    assert!(!err.to_string().contains("query=up"), "{err}");
}

#[tokio::test]
async fn unresolvable_host_is_unreachable() {
    let ep =
        PromEndpoint::new("http://prometheus.invalid:9090").with_timeout(Duration::from_secs(5));
    assert!(matches!(instant(&ep).await, PromqlError::Unreachable(_)));
}

#[tokio::test]
async fn a_slow_server_times_out() {
    let server = MockServer::start().await;
    answer(
        &server,
        json(
            200,
            r#"{"status":"success","data":{"resultType":"vector","result":[]}}"#,
        )
        .set_delay(Duration::from_secs(3)),
    )
    .await;
    let ep = endpoint(&server).with_timeout(Duration::from_millis(200));
    let started = std::time::Instant::now();
    let err = instant(&ep).await;
    assert_eq!(err, PromqlError::Timeout(Duration::from_millis(200)));
    assert!(err.is_absent());
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the deadline bounds the call"
    );
}

#[tokio::test]
async fn invalid_endpoints_and_ranges_fail_before_any_request() {
    let server = MockServer::start().await;
    let client = PromqlClient::new();
    for bad in ["", "nonsense", "ftp://x", "http://u:p@x"] {
        let err = client
            .instant_query(&PromEndpoint::new(bad), "up", None)
            .await
            .unwrap_err();
        assert!(
            matches!(err, PromqlError::InvalidEndpoint(_)),
            "{bad}: {err:?}"
        );
    }
    let ep = endpoint(&server);
    let mut zero = range();
    zero.step = Duration::ZERO;
    assert!(
        client
            .range_query(&ep, "up", zero)
            .await
            .unwrap_err()
            .is_query_error()
    );
    let mut backwards = range();
    std::mem::swap(&mut backwards.start, &mut backwards.end);
    assert!(
        client
            .range_query(&ep, "up", backwards)
            .await
            .unwrap_err()
            .is_query_error()
    );
    assert!(
        client
            .instant_query(&ep, "   ", None)
            .await
            .unwrap_err()
            .is_query_error()
    );
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "nothing was sent"
    );
}

#[tokio::test]
async fn ping_succeeds_against_a_prometheus_and_fails_otherwise() {
    let server = MockServer::start().await;
    answer(&server, json(200, r#"{"status":"success","data":{"resultType":"vector","result":[{"metric":{},"value":[1,"1"]}]}}"#)).await;
    PromqlClient::new().ping(&endpoint(&server)).await.unwrap();
    server.reset().await;
    answer(&server, ResponseTemplate::new(401)).await;
    assert!(matches!(
        PromqlClient::new().ping(&endpoint(&server)).await,
        Err(PromqlError::Unauthorized(_))
    ));
}
