//! The kube-rs source against a fake API server: request parameters, error mapping, the
//! pod-to-`PodInfo` conversion.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::future::BoxFuture;
use http::{Request, Response, StatusCode};
use jiff::Timestamp;
use kube::Client;
use kube::client::Body;
use oxikube_domain::ErrorKind;
use oxikube_ports::{LogOptions, LogPort, LogSince};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tower::Service;

use super::fake::{collect, expect_lines, texts, wire};
use crate::logs::KubeLogs;
use crate::logs::kube_source::{KubeSource, pod_info};
use crate::logs::source::{ContainerState, LogSource, OpenRequest, PodPhase, RestartPolicy};

const LOG: &str = "/api/v1/namespaces/ns/pods/p/log";
const POD: &str = "/api/v1/namespaces/ns/pods/p";

/// Serves canned bodies by path and records the query of every request.
#[derive(Clone, Default)]
struct Server {
    replies: Arc<Mutex<HashMap<String, (u16, Vec<u8>)>>>,
    queries: Arc<Mutex<Vec<(String, String)>>>,
}

impl Server {
    fn reply(&self, path: &str, status: u16, body: impl Into<Vec<u8>>) -> &Self {
        self.replies
            .lock()
            .insert(path.to_owned(), (status, body.into()));
        self
    }

    fn client(&self) -> Client {
        Client::new(self.clone(), "default")
    }

    fn query(&self, path: &str) -> String {
        self.queries
            .lock()
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, q)| q.clone())
            .expect("request recorded")
    }
}

impl Service<Request<Body>> for Server {
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let path = req.uri().path().to_owned();
        let query = req.uri().query().unwrap_or_default().to_owned();
        self.queries.lock().push((path.clone(), query));
        let (status, body) = self
            .replies
            .lock()
            .get(&path)
            .cloned()
            .unwrap_or_else(|| (404, status_json(404, "NotFound", "not found")));
        Box::pin(async move {
            Ok(Response::builder()
                .status(StatusCode::from_u16(status).expect("status"))
                .body(Body::from(body))
                .expect("response"))
        })
    }
}

fn status_json(code: u16, reason: &str, message: &str) -> Vec<u8> {
    json!({"kind": "Status", "apiVersion": "v1", "status": "Failure",
           "message": message, "reason": reason, "code": code})
    .to_string()
    .into_bytes()
}

fn request() -> OpenRequest {
    OpenRequest {
        container: "app".into(),
        follow: true,
        previous: false,
        since: None,
        tail_lines: None,
        limit_bytes: None,
    }
}

#[tokio::test]
async fn the_request_carries_every_option_and_always_asks_for_timestamps() {
    let server = Server::default();
    server.reply(LOG, 200, wire(0, "hello"));
    let source = KubeSource::new(server.client());
    let since: Timestamp = "2026-10-03T12:00:00Z".parse().unwrap();

    source
        .open(
            "ns",
            "p",
            &OpenRequest {
                since: Some(LogSince::Time(since)),
                tail_lines: Some(100),
                limit_bytes: Some(4096),
                ..request()
            },
        )
        .await
        .map(|_| ())
        .expect("open");
    let q = server.query(LOG);
    for expected in [
        "container=app",
        "follow=true",
        "timestamps=true",
        "tailLines=100",
        "limitBytes=4096",
        "sinceTime=2026-10-03T12%3A00%3A00Z",
    ] {
        assert!(q.contains(expected), "`{expected}` missing from `{q}`");
    }
    assert!(!q.contains("previous") && !q.contains("sinceSeconds"));

    let server = Server::default();
    server.reply(LOG, 200, "");
    let source = KubeSource::new(server.client());
    source
        .open(
            "ns",
            "p",
            &OpenRequest {
                follow: false,
                previous: true,
                since: Some(LogSince::Seconds(30)),
                ..request()
            },
        )
        .await
        .map(|_| ())
        .expect("open");
    let q = server.query(LOG);
    assert!(
        q.contains("previous=true") && q.contains("sinceSeconds=30"),
        "{q}"
    );
    assert!(!q.contains("follow"));
}

#[tokio::test]
async fn the_port_streams_lines_from_a_real_kube_client() {
    let server = Server::default();
    let body: Vec<u8> = (0..5).flat_map(|n| wire(n, &format!("line {n}"))).collect();
    server.reply(LOG, 200, body);
    // The pod read is best effort; a 404 there does not fail the stream.
    let logs = KubeLogs::new(server.client());
    let opts = LogOptions::default().container("app").tail_lines(5);

    let items = collect(logs.stream_logs("ns", "p", &opts).await.unwrap()).await;

    assert_eq!(texts(&items), expect_lines(0..5));
    assert!(server.query(LOG).contains("tailLines=5"));
}

#[tokio::test]
async fn api_failures_map_to_error_kinds() {
    for (code, reason, kind) in [
        (404, "NotFound", ErrorKind::NotFound),
        (403, "Forbidden", ErrorKind::Forbidden),
        (401, "Unauthorized", ErrorKind::Auth),
        (400, "BadRequest", ErrorKind::Validation),
        (503, "ServiceUnavailable", ErrorKind::Network),
    ] {
        let server = Server::default();
        server.reply(
            LOG,
            code,
            status_json(code, reason, "container is waiting to start"),
        );
        let source = KubeSource::new(server.client());
        let err = source
            .open("ns", "p", &request())
            .await
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err.kind(), kind, "HTTP {code}");
    }
}

#[tokio::test]
async fn a_missing_pod_reads_as_none() {
    let server = Server::default();
    let source = KubeSource::new(server.client());
    assert!(source.pod("ns", "p").await.unwrap().is_none());
}

fn pod_json() -> Value {
    json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {
            "name": "p", "namespace": "ns", "uid": "u-1",
            "annotations": {"kubectl.kubernetes.io/default-container": "web"},
            "deletionTimestamp": "2026-10-03T12:00:00Z",
        },
        "spec": {
            "restartPolicy": "OnFailure",
            "initContainers": [{"name": "setup"}],
            "containers": [{"name": "web"}, {"name": "sidecar"}],
            "ephemeralContainers": [{"name": "debugger"}],
        },
        "status": {
            "phase": "Running",
            "initContainerStatuses": [{"name": "setup", "restartCount": 0, "image": "i", "imageID": "",
                "ready": true, "state": {"terminated": {"exitCode": 0}}}],
            "containerStatuses": [
                {"name": "web", "restartCount": 3, "image": "i", "imageID": "", "ready": true,
                 "state": {"running": {"startedAt": "2026-10-03T12:00:00Z"}}},
                {"name": "sidecar", "restartCount": 1, "image": "i", "imageID": "", "ready": false,
                 "state": {"waiting": {"reason": "CrashLoopBackOff"}}},
            ],
            "ephemeralContainerStatuses": [{"name": "debugger", "restartCount": 0, "image": "i",
                "imageID": "", "ready": false, "state": {"terminated": {"exitCode": 137}}}],
        },
    })
}

#[tokio::test]
async fn pod_json_converts_to_the_readers_view() {
    let server = Server::default();
    server.reply(POD, 200, pod_json().to_string());
    let info = KubeSource::new(server.client())
        .pod("ns", "p")
        .await
        .unwrap()
        .expect("pod");

    assert_eq!(
        (
            info.name.as_str(),
            info.namespace.as_str(),
            info.uid.as_str()
        ),
        ("p", "ns", "u-1")
    );
    assert!(info.deleting);
    assert_eq!(info.phase, PodPhase::Running);
    assert_eq!(info.restart_policy, RestartPolicy::OnFailure);
    assert_eq!(info.default_container.as_deref(), Some("web"));
    let names: Vec<_> = info
        .containers
        .iter()
        .map(|c| (c.name.as_str(), c.init))
        .collect();
    assert_eq!(
        names,
        [
            ("setup", true),
            ("web", false),
            ("sidecar", false),
            ("debugger", false)
        ]
    );
    let state = |n: &str| info.container(n).unwrap().state;
    assert_eq!(state("setup"), ContainerState::Terminated { exit_code: 0 });
    assert_eq!(state("web"), ContainerState::Running);
    assert_eq!(state("sidecar"), ContainerState::Waiting);
    assert_eq!(
        state("debugger"),
        ContainerState::Terminated { exit_code: 137 }
    );
    assert_eq!(info.container("web").unwrap().restart_count, 3);
    assert_eq!(info.default_container_name(), Some("web"));
}

#[test]
fn a_pod_without_status_has_unknown_container_states() {
    let pod: k8s_openapi::api::core::v1::Pod = serde_json::from_value(json!({
        "metadata": {"name": "p"},
        "spec": {"containers": [{"name": "web"}]},
    }))
    .unwrap();
    let info = pod_info(&pod);
    assert_eq!(info.phase, PodPhase::Unknown);
    assert_eq!(info.restart_policy, RestartPolicy::Always);
    assert_eq!(
        info.container("web").unwrap().state,
        ContainerState::Unknown
    );
    assert!(!info.container("web").unwrap().has_started());
}
