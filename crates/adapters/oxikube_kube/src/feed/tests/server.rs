//! A scripted API server for feed tests: a `tower::Service` behind a real `kube::Client`.
//!
//! Unlike the crate's path-only `FakeApi`, it tells a list from a watch (`watch=true`) on the
//! same path. Each watch reply is a finite body of watch-event lines; when it ends, kube's
//! watcher reconnects from the last resource version and gets the next reply. With no reply
//! left, a watch hangs like an idle server; [`FeedServer::open_watches`] counts those, so a
//! test can see them cancelled when the feed is dropped.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

use futures::future::BoxFuture;
use http::{Request, Response, StatusCode};
use kube::Client;
use kube::client::Body;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tower::Service;

use crate::discovery::{DiscoveryConfig, KubeDiscovery};
use crate::fake_api::status_body;
use crate::feed::FeedConfig;
use crate::resources::KubeResources;

pub(super) const PODS: &str = "/api/v1/namespaces/default/pods";
pub(super) const OTHER_PODS: &str = "/api/v1/namespaces/other/pods";
pub(super) const ALL_PODS: &str = "/api/v1/pods";

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// One scripted reply.
#[derive(Clone)]
pub(super) enum Reply {
    /// A JSON body with a status code.
    Json(u16, Value),
    /// HTTP 200 with these watch events, one per line, then the end of the stream.
    Events(Vec<Value>),
    /// The connection fails before any response.
    Drop,
    /// No response ever, like a connect attempt to an unreachable server.
    Hang,
}

#[derive(Default)]
struct State {
    fixed: HashMap<String, Value>,
    lists: HashMap<String, VecDeque<Reply>>,
    watches: HashMap<String, VecDeque<Reply>>,
    requests: Vec<(String, String)>,
    /// `Accept-Encoding` of every watch request, oldest first.
    watch_encodings: Vec<Option<String>>,
    /// `(path, is_watch, Accept)` of every request, oldest first.
    accepts: Vec<(String, bool, Option<String>)>,
}

/// Handle to the fake server; clones share state.
#[derive(Clone, Default)]
pub(super) struct FeedServer {
    state: Arc<Mutex<State>>,
    open_watches: Arc<AtomicUsize>,
}

impl FeedServer {
    /// A server with legacy discovery for `Pod` (namespaced), `Namespace` (cluster-scoped)
    /// and `Binding` (no `watch` verb), and `/version` at `v1.<minor>`.
    pub(super) fn new(minor: u32) -> Self {
        let server = Self::default();
        let full = ["get", "list", "watch"];
        let resource = |name: &str, kind: &str, namespaced: bool, verbs: &[&str]| json!({"name": name, "singularName": "", "namespaced": namespaced, "kind": kind, "verbs": verbs});
        let mut state = server.state.lock();
        state.fixed.insert(
            "/api".into(),
            json!({"kind": "APIVersions", "versions": ["v1"]}),
        );
        state.fixed.insert(
            "/apis".into(),
            json!({"kind": "APIGroupList", "groups": []}),
        );
        state.fixed.insert(
            "/api/v1".into(),
            json!({"kind": "APIResourceList", "groupVersion": "v1", "resources": [
                resource("pods", "Pod", true, &full),
                resource("namespaces", "Namespace", false, &full),
                resource("bindings", "Binding", true, &["get", "list"]),
            ]}),
        );
        state.fixed.insert(
            "/version".into(),
            json!({"major": "1", "minor": minor.to_string(), "gitVersion": format!("v1.{minor}.0"), "platform": "linux/arm64"}),
        );
        drop(state);
        server
    }

    /// A kube client served by this fake.
    pub(super) fn client(&self) -> Client {
        Client::new(self.clone(), "default")
    }

    /// `KubeResources` over this server with `config` for its feeds.
    pub(super) fn resources(&self, config: FeedConfig) -> KubeResources {
        let discovery = KubeDiscovery::with_config(
            self.client(),
            DiscoveryConfig {
                aggregated: false,
                ..DiscoveryConfig::default()
            },
        );
        KubeResources::new(self.client(), discovery).with_feed_config(config)
    }

    /// Queues a list reply on `path`. Replies are served in order; the last one repeats.
    pub(super) fn list(&self, path: &str, reply: Reply) -> &Self {
        let mut state = self.state.lock();
        state.lists.entry(path.into()).or_default().push_back(reply);
        drop(state);
        self
    }

    /// Queues a watch reply on `path`. Each is served once; then watches hang.
    pub(super) fn watch(&self, path: &str, reply: Reply) -> &Self {
        let mut state = self.state.lock();
        state
            .watches
            .entry(path.into())
            .or_default()
            .push_back(reply);
        drop(state);
        self
    }

    /// Requests to `path` so far, as their raw query strings, oldest first.
    pub(super) fn queries(&self, path: &str) -> Vec<String> {
        let state = self.state.lock();
        state
            .requests
            .iter()
            .filter(|(p, _)| p == path)
            .map(|(_, q)| q.clone())
            .collect()
    }

    /// List requests (not watches) to `path` so far.
    pub(super) fn list_hits(&self, path: &str) -> usize {
        self.queries(path).iter().filter(|q| !is_watch(q)).count()
    }

    /// The `Accept-Encoding` header of every watch request so far, oldest first.
    pub(super) fn watch_encodings(&self) -> Vec<Option<String>> {
        self.state.lock().watch_encodings.clone()
    }

    /// The `Accept` header of every request to `path` so far as `(is_watch, header)`, oldest first.
    pub(super) fn accepts(&self, path: &str) -> Vec<(bool, Option<String>)> {
        let state = self.state.lock();
        state
            .accepts
            .iter()
            .filter(|(p, ..)| p == path)
            .map(|(_, watch, accept)| (*watch, accept.clone()))
            .collect()
    }

    /// Requests currently hanging on the server: watches with no reply left, and
    /// [`Reply::Hang`].
    pub(super) fn open_watches(&self) -> usize {
        self.open_watches.load(Ordering::SeqCst)
    }

    fn route(
        &self,
        path: &str,
        query: &str,
        encoding: Option<String>,
        accept: Option<String>,
    ) -> Option<Reply> {
        let mut state = self.state.lock();
        state.requests.push((path.into(), query.into()));
        state.accepts.push((path.into(), is_watch(query), accept));
        if is_watch(query) {
            state.watch_encodings.push(encoding);
        }
        if let Some(body) = state.fixed.get(path) {
            return Some(Reply::Json(200, body.clone()));
        }
        if is_watch(query) {
            // `None` means: hang.
            return state.watches.get_mut(path).and_then(VecDeque::pop_front);
        }
        let reply = match state.lists.get_mut(path) {
            Some(queue) if queue.len() > 1 => queue.pop_front(),
            Some(queue) => queue.front().cloned(),
            None => None,
        };
        Some(reply.unwrap_or_else(|| Reply::Json(404, status_body(404, "NotFound", "no reply"))))
    }
}

fn is_watch(query: &str) -> bool {
    query.split('&').any(|pair| pair == "watch=true")
}

/// Counts a hanging watch while it is alive.
struct OpenWatch(Arc<AtomicUsize>);

impl OpenWatch {
    fn new(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(counter.clone())
    }
}

impl Drop for OpenWatch {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn respond(code: u16, body: String) -> Response<Body> {
    Response::builder()
        .status(StatusCode::from_u16(code).expect("status code"))
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.into_bytes()))
        .expect("response")
}

impl Service<Request<Body>> for FeedServer {
    type Response = Response<Body>;
    type Error = BoxError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let path = req.uri().path().to_owned();
        let query = req.uri().query().unwrap_or_default().to_owned();
        let encoding = req
            .headers()
            .get(http::header::ACCEPT_ENCODING)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let accept = req
            .headers()
            .get(http::header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let reply = self.route(&path, &query, encoding, accept);
        let counter = self.open_watches.clone();
        Box::pin(async move {
            match reply {
                None | Some(Reply::Hang) => {
                    let _open = OpenWatch::new(&counter);
                    std::future::pending::<()>().await;
                    unreachable!("a pending future never completes")
                }
                Some(Reply::Json(code, body)) => Ok(respond(code, body.to_string())),
                Some(Reply::Events(events)) => {
                    let lines: Vec<String> = events.iter().map(Value::to_string).collect();
                    Ok(respond(200, lines.join("\n") + "\n"))
                }
                Some(Reply::Drop) => Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "connection reset by peer",
                )) as BoxError),
            }
        })
    }
}

/// A pod with `uid` and `resourceVersion` in `namespace`, as a list item (no type fields).
pub(super) fn pod_in(namespace: &str, name: &str, uid: &str, rv: &str) -> Value {
    json!({
        "metadata": {
            "name": name, "namespace": namespace, "uid": uid, "resourceVersion": rv,
            "managedFields": [{"manager": "kubectl", "operation": "Update"}],
        },
        "spec": {"containers": [{"name": "c", "image": "busybox"}]},
    })
}

/// A pod as `PartialObjectMetadata` in `namespace`, as a list item: no `spec`, no type fields.
pub(super) fn meta_pod_in(namespace: &str, name: &str, uid: &str, rv: &str) -> Value {
    json!({
        "metadata": {
            "name": name, "namespace": namespace, "uid": uid, "resourceVersion": rv,
            "labels": {"app": "web"},
            "ownerReferences": [{"apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "web-1",
                "uid": "rs-1", "controller": true}],
            "managedFields": [{"manager": "kubectl", "operation": "Update"}],
        },
    })
}

/// A metadata-only pod in `default`.
pub(super) fn meta_pod(name: &str, uid: &str, rv: &str) -> Value {
    meta_pod_in("default", name, uid, rv)
}

/// `object` with the `v1` `Pod` type fields a `get` reply carries.
pub(super) fn typed_pod(mut object: Value) -> Value {
    object["apiVersion"] = json!("v1");
    object["kind"] = json!("Pod");
    object
}

/// A watch event of `kind` for a metadata-only `object`, typed as the server types them.
pub(super) fn meta_event(kind: &str, mut object: Value) -> Value {
    object["apiVersion"] = json!("meta.k8s.io/v1");
    object["kind"] = json!("PartialObjectMetadata");
    json!({"type": kind, "object": object})
}

/// A pod in `default`.
pub(super) fn pod(name: &str, uid: &str, rv: &str) -> Value {
    pod_in("default", name, uid, rv)
}

/// A `PodList` with `items` at collection version `rv`.
pub(super) fn pod_list(items: Vec<Value>, rv: &str) -> Reply {
    Reply::Json(
        200,
        json!({"kind": "PodList", "apiVersion": "v1", "metadata": {"resourceVersion": rv}, "items": items}),
    )
}

/// A watch event of `kind` (`ADDED`, `MODIFIED`, `DELETED`) for `object`.
pub(super) fn event(kind: &str, mut object: Value) -> Value {
    object["apiVersion"] = json!("v1");
    object["kind"] = json!("Pod");
    json!({"type": kind, "object": object})
}

/// The bookmark that ends a streaming list's initial events.
pub(super) fn initial_events_end(rv: &str) -> Value {
    json!({"type": "BOOKMARK", "object": {"kind": "Pod", "apiVersion": "v1", "metadata": {
        "resourceVersion": rv, "annotations": {"k8s.io/initial-events-end": "true"}}}})
}

/// A plain bookmark at `rv`: the watch is alive but nothing changed.
pub(super) fn bookmark(rv: &str) -> Value {
    json!({"type": "BOOKMARK", "object": {"kind": "Pod", "apiVersion": "v1", "metadata": {
        "resourceVersion": rv}}})
}

/// A watch `ERROR` event carrying HTTP `code`.
pub(super) fn error_event(code: u16, reason: &str) -> Value {
    json!({"type": "ERROR", "object": status_body(code, reason, "watch failed")})
}
