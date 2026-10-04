//! A scriptable in-process API server for unit tests.
//!
//! [`FakeApi`] is a `tower::Service` behind a real `kube::Client` (`Client::new(svc, ..)`),
//! so a test exercises kube's request building, response parsing and error mapping with no
//! cluster and no network. Each path gets a queue of scripted replies; requests are
//! recorded with their method and JSON body. The discovery tests keep their own
//! fixture-backed server (`discovery/tests/fake.rs`), which models discovery's `Accept`
//! negotiation on top of the same idea.

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::future::BoxFuture;
use http::{Method, Request, Response, StatusCode};
use kube::Client;
use kube::client::Body;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tower::Service;

/// One request the server received.
#[derive(Debug, Clone)]
pub(crate) struct Recorded {
    pub(crate) method: Method,
    pub(crate) path: String,
    /// The JSON body, if the request had one.
    pub(crate) body: Option<Value>,
}

#[derive(Default)]
struct State {
    replies: HashMap<String, VecDeque<(u16, Value)>>,
    requests: Vec<Recorded>,
}

/// Handle to the fake server; clones share state.
#[derive(Clone, Default)]
pub(crate) struct FakeApi {
    state: Arc<Mutex<State>>,
}

impl FakeApi {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A kube client whose every request is served by this fake.
    pub(crate) fn client(&self) -> Client {
        Client::new(self.clone(), "default")
    }

    /// Queues a reply for `path` (any method). Replies are served in order and the last
    /// one repeats; a path with no reply answers 404.
    pub(crate) fn reply(&self, path: &str, status: u16, body: Value) -> &Self {
        self.state
            .lock()
            .replies
            .entry(path.to_owned())
            .or_default()
            .push_back((status, body));
        self
    }

    /// Requests received so far, oldest first.
    pub(crate) fn requests(&self) -> Vec<Recorded> {
        self.state.lock().requests.clone()
    }

    /// How many requests went to `path`.
    pub(crate) fn hits(&self, path: &str) -> usize {
        self.state
            .lock()
            .requests
            .iter()
            .filter(|r| r.path == path)
            .count()
    }

    fn respond(&self, recorded: Recorded) -> (u16, Value) {
        let mut state = self.state.lock();
        let reply = match state.replies.get_mut(&recorded.path) {
            Some(queue) if queue.len() > 1 => queue.pop_front(),
            Some(queue) => queue.front().cloned(),
            None => None,
        };
        state.requests.push(recorded);
        reply.unwrap_or_else(|| (404, status_body(404, "NotFound", "no reply scripted")))
    }
}

/// A `/version` body with `git_version`.
pub(crate) fn version_body(git_version: &str) -> Value {
    json!({"major": "1", "minor": "35", "gitVersion": git_version, "platform": "linux/arm64"})
}

/// A `metav1.Status` failure body, as the apiserver sends with an error code.
pub(crate) fn status_body(code: u16, reason: &str, message: &str) -> Value {
    json!({
        "kind": "Status", "apiVersion": "v1", "metadata": {}, "status": "Failure",
        "message": message, "reason": reason, "code": code,
    })
}

impl Service<Request<Body>> for FakeApi {
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let server = self.clone();
        Box::pin(async move {
            let (parts, body) = req.into_parts();
            let bytes = body.collect_bytes().await.expect("request body");
            let recorded = Recorded {
                method: parts.method,
                path: parts.uri.path().to_owned(),
                body: serde_json::from_slice(&bytes).ok(),
            };
            let (code, body) = server.respond(recorded);
            let response = Response::builder()
                .status(StatusCode::from_u16(code).expect("status code"))
                .header(http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string().into_bytes()))
                .expect("response");
            Ok(response)
        })
    }
}
