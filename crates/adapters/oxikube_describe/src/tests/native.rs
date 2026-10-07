//! The native backend over a kube client served by a recorded object (no cluster).

use std::sync::{Arc, Mutex};

use kube::Client;
use kube::client::Body;
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_ports::{DescribePort, DescribeSource};
use oxikube_testkit::fixtures;
use serde_json::{Value, json};

use super::{cluster, discovery, pod_ref};
use crate::NativeDescribe;

/// A client that serves the fixture pod, an empty event list, and 404 for anything else; the
/// paths asked for are recorded.
fn client(pod: Value) -> (Client, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let client = Client::new(
        tower::service_fn(move |request: http::Request<Body>| {
            let path = request.uri().path().to_owned();
            log.lock().unwrap().push(path.clone());
            let (status, body) = if path.ends_with("/pods/web-running") {
                (200, pod.clone())
            } else if path.ends_with("/events") {
                (
                    200,
                    json!({"apiVersion": "v1", "kind": "EventList", "items": []}),
                )
            } else {
                (
                    404,
                    json!({"apiVersion": "v1", "kind": "Status", "status": "Failure",
                        "reason": "NotFound", "code": 404,
                        "message": format!("{path} not found")}),
                )
            };
            async move {
                Ok::<_, std::convert::Infallible>(
                    http::Response::builder()
                        .status(status)
                        .body(Body::from(body.to_string().into_bytes()))
                        .unwrap(),
                )
            }
        }),
        "demo",
    );
    (client, seen)
}

#[tokio::test]
async fn a_recorded_pod_is_described_natively() {
    let (client, seen) = client(fixtures::pod_running().into_json());
    let describer = NativeDescribe::new(client, discovery());
    let output = describer.describe(&pod_ref()).await.unwrap();
    assert_eq!(output.source, DescribeSource::Native);
    assert!(output.text.contains("Name:"), "{}", output.text);
    assert!(output.text.contains("web-running"), "{}", output.text);
    assert!(output.text.contains("Namespace:"), "{}", output.text);
    assert!(output.text.contains("demo"), "{}", output.text);
    assert!(
        output.text.contains("registry.example/app:1.0"),
        "{}",
        output.text
    );
    // Read-only: only GETs of the object and its events.
    let paths = seen.lock().unwrap().clone();
    assert!(
        paths
            .iter()
            .any(|p| p == "/api/v1/namespaces/demo/pods/web-running"),
        "{paths:?}"
    );
}

#[tokio::test]
async fn a_missing_object_is_not_found() {
    let (client, _) = client(fixtures::pod_running().into_json());
    let describer = NativeDescribe::new(client, discovery());
    let missing = ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "demo", "nope");
    let error = describer.describe(&missing).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound, "{error}");
}

#[tokio::test]
async fn a_kind_the_cluster_does_not_serve_is_unsupported() {
    let (client, seen) = client(fixtures::pod_running().into_json());
    let describer = NativeDescribe::new(client, discovery());
    let unknown =
        ResourceRef::namespaced(cluster(), Gvk::new("x.dev", "v1", "Gadget"), "demo", "g");
    let error = describer.describe(&unknown).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported, "{error}");
    assert!(
        seen.lock().unwrap().is_empty(),
        "no request for an unknown kind"
    );
}

#[tokio::test]
async fn a_custom_resource_gets_the_generic_layout() {
    let widget = json!({
        "apiVersion": "test.oxikube.dev/v1", "kind": "Widget",
        "metadata": {"name": "w1", "namespace": "demo", "uid": "u1"},
        "spec": {"size": "large"},
    });
    let seen_path = Arc::new(Mutex::new(String::new()));
    let log = seen_path.clone();
    let client = Client::new(
        tower::service_fn(move |request: http::Request<Body>| {
            let path = request.uri().path().to_owned();
            let body = if path.ends_with("/events") {
                json!({"apiVersion": "v1", "kind": "EventList", "items": []})
            } else {
                *log.lock().unwrap() = path;
                widget.clone()
            };
            async move {
                Ok::<_, std::convert::Infallible>(
                    http::Response::builder()
                        .status(200)
                        .body(Body::from(body.to_string().into_bytes()))
                        .unwrap(),
                )
            }
        }),
        "demo",
    );
    let describer = NativeDescribe::new(client, discovery());
    let target = ResourceRef::namespaced(
        cluster(),
        Gvk::new("test.oxikube.dev", "v1", "Widget"),
        "demo",
        "w1",
    );
    let output = describer.describe(&target).await.unwrap();
    assert!(output.text.contains("w1"), "{}", output.text);
    // The plural came from discovery, not from guessing.
    assert_eq!(
        *seen_path.lock().unwrap(),
        "/apis/test.oxikube.dev/v1/namespaces/demo/widgets/w1"
    );
}

#[tokio::test]
async fn a_service_account_token_secret_is_described_without_its_token() {
    const TOKEN: &str = "eyJhbGciOiJSUzI1NiJ9.c2VjcmV0.signature";
    let secret = json!({
        "apiVersion": "v1", "kind": "Secret", "type": "kubernetes.io/service-account-token",
        "metadata": {"name": "sa-token", "namespace": "demo", "uid": "u2"},
        // base64 of the token and of a short CA and namespace
        "data": {
            "token": "ZXlKaGJHY2lPaUpTVXpJMU5pSjkuYzJWamNtVjAuc2lnbmF0dXJl",
            "ca.crt": "Y2E=",
            "namespace": "ZGVtbw==",
        },
    });
    let client = Client::new(
        tower::service_fn(move |request: http::Request<Body>| {
            let body = if request.uri().path().ends_with("/events") {
                json!({"apiVersion": "v1", "kind": "EventList", "items": []})
            } else {
                secret.clone()
            };
            async move {
                Ok::<_, std::convert::Infallible>(
                    http::Response::builder()
                        .status(200)
                        .body(Body::from(body.to_string().into_bytes()))
                        .unwrap(),
                )
            }
        }),
        "demo",
    );
    let describer = NativeDescribe::new(client, discovery());
    let target =
        ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Secret"), "demo", "sa-token");
    let output = describer.describe(&target).await.unwrap();
    assert!(output.text.contains("sa-token"), "{}", output.text);
    assert!(!output.text.contains("eyJhbGci"), "{}", output.text);
    assert!(!output.text.contains(TOKEN), "{}", output.text);
    assert!(output.text.contains("(hidden)"), "{}", output.text);
    assert!(output.text.contains("2 bytes"), "{}", output.text);
}
