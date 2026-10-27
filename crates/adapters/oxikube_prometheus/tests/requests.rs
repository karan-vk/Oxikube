//! What the client sends: auth headers, URL shape, parameters, POST for long queries.

mod common;

use common::{endpoint, range};
use jiff::Timestamp;
use oxikube_ports::secrets::SecretString;
use oxikube_ports::{
    PromAuth, PromEndpoint, PromSecret, PromqlEndpointPort, PromqlError, SecretKey, SecretStorePort,
};
use oxikube_prometheus::PromqlClient;
use oxikube_testkit::FakeSecretStorePort;
use std::sync::Arc;
use wiremock::matchers::{body_string_contains, header, header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const EMPTY: &str = r#"{"status":"success","data":{"resultType":"vector","result":[]}}"#;

fn ok() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(EMPTY.as_bytes().to_vec(), "application/json")
}

fn secret(value: &str) -> PromSecret {
    PromSecret::Memory(SecretString::from(value.to_owned()))
}

async fn only_request(server: &MockServer) -> Request {
    let mut requests = server
        .received_requests()
        .await
        .expect("the mock server records requests");
    assert_eq!(requests.len(), 1, "exactly one request: {requests:?}");
    requests.remove(0)
}

#[tokio::test]
async fn bearer_auth_sends_the_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("authorization", "Bearer tok-123"))
        .respond_with(ok())
        .expect(1)
        .mount(&server)
        .await;
    // A token stored with a trailing newline still works.
    let ep = endpoint(&server).with_auth(PromAuth::Bearer(secret("tok-123\n")));
    PromqlClient::new()
        .instant_query(&ep, "up", None)
        .await
        .unwrap();
}

#[tokio::test]
async fn basic_auth_sends_user_and_password() {
    let server = MockServer::start().await;
    // base64("alice:s3cret")
    Mock::given(method("GET"))
        .and(header("authorization", "Basic YWxpY2U6czNjcmV0"))
        .respond_with(ok())
        .expect(1)
        .mount(&server)
        .await;
    let ep = endpoint(&server).with_auth(PromAuth::Basic {
        username: "alice".into(),
        password: secret("s3cret"),
    });
    PromqlClient::new()
        .instant_query(&ep, "up", None)
        .await
        .unwrap();
}

#[tokio::test]
async fn no_auth_sends_no_authorization_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ok())
        .mount(&server)
        .await;
    PromqlClient::new()
        .instant_query(&endpoint(&server), "up", None)
        .await
        .unwrap();
    let request = only_request(&server).await;
    assert!(!request.headers.contains_key("authorization"));
}

#[tokio::test]
async fn a_rejected_credential_is_unauthorized() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header_exists("authorization"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let ep = endpoint(&server).with_auth(PromAuth::Bearer(secret("bad")));
    let err = PromqlClient::new()
        .instant_query(&ep, "up", None)
        .await
        .unwrap_err();
    assert!(matches!(err, PromqlError::Unauthorized(_)), "{err:?}");
}

#[tokio::test]
async fn keychain_credentials_are_read_per_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("authorization", "Bearer from-keychain"))
        .respond_with(ok())
        .mount(&server)
        .await;
    let store = Arc::new(FakeSecretStorePort::new());
    let key = SecretKey::new("prometheus", "cluster-a").unwrap();
    store
        .set(&key, SecretString::from("from-keychain".to_owned()))
        .await
        .unwrap();
    let client = PromqlClient::with_secret_store(store.clone());
    let ep = endpoint(&server).with_auth(PromAuth::Bearer(PromSecret::Keychain(key.clone())));

    client.instant_query(&ep, "up", None).await.unwrap();

    // Rotating the secret is picked up by the next call.
    store
        .set(&key, SecretString::from("rotated".to_owned()))
        .await
        .unwrap();
    server.reset().await;
    Mock::given(method("GET"))
        .and(header("authorization", "Bearer rotated"))
        .respond_with(ok())
        .mount(&server)
        .await;
    client.instant_query(&ep, "up", None).await.unwrap();
}

#[tokio::test]
async fn a_missing_or_unreadable_keychain_entry_is_unauthorized_and_sends_nothing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ok())
        .mount(&server)
        .await;
    let key = SecretKey::new("prometheus", "absent").unwrap();
    let ep = endpoint(&server).with_auth(PromAuth::Bearer(PromSecret::Keychain(key)));

    let empty = PromqlClient::with_secret_store(Arc::new(FakeSecretStorePort::new()));
    assert!(matches!(
        empty.instant_query(&ep, "up", None).await,
        Err(PromqlError::Unauthorized(_))
    ));

    // No store at all.
    assert!(matches!(
        PromqlClient::new().instant_query(&ep, "up", None).await,
        Err(PromqlError::Unauthorized(_))
    ));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn path_prefixes_and_slashes() {
    let cases = [
        ("", None, "/api/v1/query"),
        ("/", None, "/api/v1/query"),
        ("", Some("/prometheus"), "/prometheus/api/v1/query"),
        ("/", Some("prometheus/"), "/prometheus/api/v1/query"),
        (
            "/grafana/",
            Some("/api/datasources/proxy/1/"),
            "/grafana/api/datasources/proxy/1/api/v1/query",
        ),
    ];
    for (base_path, prefix, want) in cases {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(want))
            .respond_with(ok())
            .expect(1)
            .mount(&server)
            .await;
        let mut ep = PromEndpoint::new(format!("{}{base_path}", server.uri()));
        ep.path_prefix = prefix.map(str::to_owned);
        PromqlClient::new()
            .instant_query(&ep, "up", None)
            .await
            .unwrap_or_else(|e| panic!("{want}: {e}"));
    }
}

#[tokio::test]
async fn an_existing_query_string_on_the_base_url_survives() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/p/api/v1/query"))
        .and(query_param("org", "7"))
        .and(query_param("query", "up"))
        .respond_with(ok())
        .expect(1)
        .mount(&server)
        .await;
    let ep = PromEndpoint::new(format!("{}/?org=7", server.uri())).with_path_prefix("/p");
    PromqlClient::new()
        .instant_query(&ep, "up", None)
        .await
        .unwrap();
}

#[tokio::test]
async fn instant_time_and_range_parameters() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/query"))
        .and(query_param("time", "1700000000.123"))
        .respond_with(ok())
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/query_range"))
        .and(query_param("start", "1791707595.000"))
        .and(query_param("end", "1791707599.000"))
        .and(query_param("step", "2"))
        .respond_with(ok())
        .expect(1)
        .mount(&server)
        .await;
    let client = PromqlClient::new();
    let at = Timestamp::from_millisecond(1_700_000_000_123).unwrap();
    client
        .instant_query(&endpoint(&server), "up", Some(at))
        .await
        .unwrap();
    client
        .range_query(&endpoint(&server), "up", range())
        .await
        .unwrap();
}

#[tokio::test]
async fn awkward_query_text_is_encoded() {
    let server = MockServer::start().await;
    let query = r#"sum by (pod) (rate(container_cpu_usage_seconds_total{namespace="a&b",pod=~"x+ y"}[5m])) # é"#;
    Mock::given(method("GET"))
        .and(query_param("query", query))
        .respond_with(ok())
        .expect(1)
        .mount(&server)
        .await;
    PromqlClient::new()
        .instant_query(&endpoint(&server), query, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_very_long_query_goes_in_a_post_body() {
    let server = MockServer::start().await;
    let query = format!("sum({})", vec!["up"; 2000].join(" + "));
    assert!(query.len() > 5000);
    Mock::given(method("POST"))
        .and(path("/api/v1/query"))
        .and(body_string_contains("query=sum%28up"))
        .respond_with(ok())
        .expect(1)
        .mount(&server)
        .await;
    PromqlClient::new()
        .instant_query(&endpoint(&server), &query, None)
        .await
        .unwrap();
}
