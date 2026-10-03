//! An in-process API server for `kube::Client`: serves the discovery fixtures, records requests
//! and can be told to fail.

use std::collections::HashMap;
use std::convert::Infallible;
use std::future::{Ready, ready};
use std::sync::Arc;
use std::task::{Context, Poll};

use http::{Request, Response, StatusCode};
use kube::Client;
use kube::client::Body;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tower::Service;

use crate::discovery::KubeDiscovery;

const AGGREGATED: &str = include_str!("../../../tests/fixtures/discovery/aggregated.json");
const LEGACY: &str = include_str!("../../../tests/fixtures/discovery/legacy.json");

/// How the server treats the aggregated `Accept` header on `/api` and `/apis`.
#[derive(Clone, Copy)]
pub(super) enum Behaviour {
    /// Kubernetes 1.26+: serves `apidiscovery.k8s.io/v2` when asked.
    Aggregated,
    /// Older servers: ignore the header and serve the legacy documents.
    IgnoreAccept,
    /// The aggregated request fails with this status.
    AggregatedFails(u16),
}

struct State {
    behaviour: Behaviour,
    aggregated: Value,
    legacy: HashMap<String, Value>,
    failing: HashMap<String, u16>,
    requests: Vec<String>,
}

/// Handle to the fake server; clone freely.
#[derive(Clone)]
pub(super) struct FakeApiServer {
    state: Arc<Mutex<State>>,
}

impl FakeApiServer {
    pub(super) fn new(behaviour: Behaviour) -> Self {
        let legacy: HashMap<String, Value> = serde_json::from_str(LEGACY).expect("legacy fixture");
        Self {
            state: Arc::new(Mutex::new(State {
                behaviour,
                aggregated: serde_json::from_str(AGGREGATED).expect("aggregated fixture"),
                legacy,
                failing: HashMap::new(),
                requests: Vec::new(),
            })),
        }
    }

    pub(super) fn client(&self) -> Client {
        Client::new(self.clone(), "default")
    }

    pub(super) fn discovery(&self) -> KubeDiscovery {
        KubeDiscovery::new(self.client())
    }

    /// Requests seen so far as `"<path>"` (legacy) or `"<path> (aggregated)"`.
    pub(super) fn requests(&self) -> Vec<String> {
        self.state.lock().requests.clone()
    }

    pub(super) fn request_count(&self) -> usize {
        self.state.lock().requests.len()
    }

    /// Makes every request for `path` fail with `status`.
    pub(super) fn fail_path(&self, path: &str, status: u16) {
        self.state.lock().failing.insert(path.to_owned(), status);
    }

    /// Serves a new aggregated group (and its legacy documents), like a CRD being established.
    pub(super) fn add_group(&self, group: &str, version: &str, kind: &str, plural: &str) {
        let mut state = self.state.lock();
        let resource = json!({
            "resource": plural,
            "responseKind": {"group": "", "version": "", "kind": kind},
            "scope": "Namespaced",
            "verbs": ["get", "list", "watch", "create"],
        });
        let item = json!({
            "metadata": {"name": group},
            "versions": [{"version": version, "resources": [resource], "freshness": "Current"}],
        });
        if let Some(items) = state.aggregated["apis"]["items"].as_array_mut() {
            items.push(item);
        }
        let group_version = format!("{group}/{version}");
        let groups = &mut state.legacy.get_mut("/apis").expect("/apis")["groups"];
        if let Some(groups) = groups.as_array_mut() {
            groups.push(json!({
                "name": group,
                "versions": [{"groupVersion": group_version, "version": version}],
                "preferredVersion": {"groupVersion": group_version, "version": version},
            }));
        }
        state.legacy.insert(
            format!("/apis/{group_version}"),
            json!({
                "kind": "APIResourceList",
                "groupVersion": group_version,
                "resources": [{
                    "name": plural, "singularName": "", "namespaced": true, "kind": kind,
                    "verbs": ["get", "list", "watch", "create"],
                }],
            }),
        );
    }

    fn respond(&self, path: &str, wants_aggregated: bool) -> (StatusCode, Value) {
        let mut state = self.state.lock();
        state.requests.push(if wants_aggregated {
            format!("{path} (aggregated)")
        } else {
            path.to_owned()
        });
        if let Some(code) = state.failing.get(path) {
            return status(*code);
        }
        if path == "/version" {
            return (
                StatusCode::OK,
                json!({"major": "1", "minor": "34", "gitVersion": "v1.34.1", "platform": "linux/arm64"}),
            );
        }
        let aggregated_path = path == "/api" || path == "/apis";
        if aggregated_path && wants_aggregated {
            match state.behaviour {
                Behaviour::Aggregated => {
                    let key = path.trim_start_matches('/');
                    return (StatusCode::OK, state.aggregated[key].clone());
                }
                Behaviour::AggregatedFails(code) => return status(code),
                Behaviour::IgnoreAccept => {}
            }
        }
        match state.legacy.get(path) {
            Some(doc) => (StatusCode::OK, doc.clone()),
            None => status(404),
        }
    }
}

fn status(code: u16) -> (StatusCode, Value) {
    let http_status = StatusCode::from_u16(code).expect("status code");
    let body = json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure",
        "message": format!("fake server says {code}"), "reason": "Fake", "code": code,
    });
    (http_status, body)
}

impl Service<Request<Body>> for FakeApiServer {
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let wants_aggregated = req
            .headers()
            .get(http::header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("apidiscovery.k8s.io"));
        let (code, body) = self.respond(req.uri().path(), wants_aggregated);
        let response = Response::builder()
            .status(code)
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string().into_bytes()))
            .expect("response");
        ready(Ok(response))
    }
}
