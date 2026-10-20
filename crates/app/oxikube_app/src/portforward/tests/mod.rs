//! The port-forward manager's tests: the pure state machine and resolution, then the manager over
//! a fake backend (watch snapshots pushed by the test), the testkit's `FakePortForwardPort` (a
//! real loopback echo listener), a manual clock and the notification hooks' fake.

mod agent_tools;
mod commands;
mod manager;
mod resolve;
mod starter;
mod state;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use futures::channel::mpsc;
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::portforward::{ForwardPort, ForwardSpec};
use oxikube_ports::PortForwardPort;
use oxikube_testkit::{FakeClockPort, FakeNotificationSourcePort, FakePortForwardPort, resource};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::backend::{ForwardBackend, ObjectWatch};
use super::manager::PortForwardManager;
use super::model::{ForwardId, ForwardInfo};
use crate::store::{Spawner, StoreQuery};

pub(super) const NS: &str = "shop";

pub(super) fn cluster() -> ClusterId {
    ClusterId::new("kubeconfig", &ContextName::from("kind"))
}

pub(super) fn target(kind: &str, name: &str) -> ResourceRef {
    ResourceRef::new(cluster(), Gvk::new("", "v1", kind), Some(NS.into()), name)
}

pub(super) fn pod_spec(name: &str, port: u16) -> ForwardSpec {
    ForwardSpec::new(target("Pod", name), ForwardPort::Number(port))
}

pub(super) fn service_spec(name: &str, port: u16) -> ForwardSpec {
    ForwardSpec::new(target("Service", name), ForwardPort::Number(port))
}

/// A pod in `phase` with one container port `http: 8080`.
pub(super) fn pod(name: &str, phase: &str) -> Resource {
    resource("v1", "Pod")
        .name(name)
        .namespace(NS)
        .field(
            "spec",
            json!({"containers": [{"name": "app", "ports": [{"name": "http", "containerPort": 8080}]}]}),
        )
        .field("status", json!({ "phase": phase }))
        .build()
}

/// A Service `name` with `ports` (each `{"name", "port", "targetPort"}`).
pub(super) fn service(name: &str, ports: Value) -> Resource {
    resource("v1", "Service")
        .name(name)
        .namespace(NS)
        .field("spec", json!({ "selector": {"app": name}, "ports": ports }))
        .build()
}

/// An EndpointSlice of Service `service` with `ports` and `(pod, ready)` endpoints.
pub(super) fn slice(
    name: &str,
    service: &str,
    ports: Value,
    endpoints: &[(&str, bool)],
) -> Resource {
    let endpoints: Vec<Value> = endpoints
        .iter()
        .enumerate()
        .map(|(i, (pod, ready))| {
            json!({
                "addresses": [format!("10.0.0.{}", i + 1)],
                "conditions": { "ready": ready },
                "targetRef": { "kind": "Pod", "name": pod, "namespace": NS },
            })
        })
        .collect();
    resource("discovery.k8s.io/v1", "EndpointSlice")
        .name(name)
        .namespace(NS)
        .label("kubernetes.io/service-name", service)
        .field("addressType", json!("IPv4"))
        .field("ports", ports)
        .field("endpoints", json!(endpoints))
        .build()
}

/// The watch side the test drives: every watch of a kind gets each snapshot pushed for it, and a
/// new watch starts from the last one (as a store subscription starts from the cache).
#[derive(Default)]
pub(super) struct FakeBackend {
    pub port: Option<Arc<FakePortForwardPort>>,
    /// Replaces `port` when set (a test's own port, e.g. one with a graceful stop).
    pub port_override: Mutex<Option<Arc<dyn PortForwardPort>>>,
    pub read_only: Mutex<bool>,
    /// Every query a watch was opened with.
    pub queries: Mutex<Vec<StoreQuery>>,
    feeds: Mutex<HashMap<String, Feed>>,
}

#[derive(Default)]
struct Feed {
    last: Option<Vec<Resource>>,
    watchers: Vec<mpsc::UnboundedSender<Result<Vec<Resource>, String>>>,
}

impl FakeBackend {
    /// Publishes `objects` as the current list of `kind`.
    pub fn push(&self, kind: &str, objects: Vec<Resource>) {
        let mut feeds = self.feeds.lock();
        let feed = feeds.entry(kind.to_owned()).or_default();
        feed.watchers
            .retain(|w| w.unbounded_send(Ok(objects.clone())).is_ok());
        feed.last = Some(objects);
    }

    /// How many watches of `kind` are open.
    pub fn watchers(&self, kind: &str) -> usize {
        let mut feeds = self.feeds.lock();
        let feed = feeds.entry(kind.to_owned()).or_default();
        feed.watchers.retain(|w| !w.is_closed());
        feed.watchers.len()
    }
}

impl ForwardBackend for FakeBackend {
    fn port_forward(&self, _: &ClusterId) -> Option<Arc<dyn PortForwardPort>> {
        if let Some(port) = self.port_override.lock().clone() {
            return Some(port);
        }
        self.port.clone().map(|p| p as Arc<dyn PortForwardPort>)
    }

    fn is_read_only(&self, _: &ClusterId) -> bool {
        *self.read_only.lock()
    }

    fn watch(&self, _: &ClusterId, query: StoreQuery) -> Option<ObjectWatch> {
        let (tx, rx) = mpsc::unbounded();
        let mut feeds = self.feeds.lock();
        let feed = feeds.entry(query.gvk.kind.to_string()).or_default();
        if let Some(last) = &feed.last {
            let _ = tx.unbounded_send(Ok(last.clone()));
        }
        feed.watchers.push(tx);
        drop(feeds);
        self.queries.lock().push(query.clone());
        let name = query.filter.name;
        let labels = query.filter.labels;
        Some(
            rx.map(move |item| {
                item.map(|objects| {
                    objects
                        .into_iter()
                        .filter(|o| name.as_deref().is_none_or(|n| &*o.meta.name == n))
                        .filter(|o| labels.as_ref().is_none_or(|l| l.matches(&o.meta.labels)))
                        .collect()
                })
            })
            .boxed(),
        )
    }
}

/// A manager over a fake backend, port, clock and notification hooks.
pub(super) struct Rig {
    pub manager: PortForwardManager,
    pub backend: Arc<FakeBackend>,
    pub port: Arc<FakePortForwardPort>,
    pub clock: Arc<FakeClockPort>,
    pub notes: Arc<FakeNotificationSourcePort>,
}

impl Rig {
    pub fn new() -> Self {
        let port = Arc::new(FakePortForwardPort::new());
        let backend = Arc::new(FakeBackend {
            port: Some(port.clone()),
            ..FakeBackend::default()
        });
        let clock = Arc::new(FakeClockPort::default());
        let notes = Arc::new(FakeNotificationSourcePort::new());
        let spawner: Arc<dyn Spawner> =
            Arc::new(|task: futures::future::BoxFuture<'static, ()>| {
                tokio::spawn(task);
            });
        let manager =
            PortForwardManager::new(backend.clone(), spawner, clock.clone(), Some(notes.clone()));
        Self {
            manager,
            backend,
            port,
            clock,
            notes,
        }
    }

    /// Forward `id` once `done` holds for it, polled on real time (the IO is real) for up to 5 s.
    pub async fn until(
        &self,
        id: ForwardId,
        mut done: impl FnMut(&ForwardInfo) -> bool,
    ) -> ForwardInfo {
        for _ in 0..1000 {
            if let Some(info) = self.manager.get(id).filter(|i| done(i)) {
                return info;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("forward {id} never got there: {:?}", self.manager.get(id));
    }

    /// Advances the manual clock by `step` until `done` holds (at most `limit`); returns the fake
    /// time it took.
    pub async fn advance_until(
        &self,
        id: ForwardId,
        step: Duration,
        limit: Duration,
        mut done: impl FnMut(&ForwardInfo) -> bool,
    ) -> Duration {
        let mut elapsed = Duration::ZERO;
        loop {
            for _ in 0..20 {
                if self.manager.get(id).is_some_and(|i| done(&i)) {
                    return elapsed;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            assert!(
                elapsed < limit,
                "not done after {elapsed:?}: {:?}",
                self.manager.get(id)
            );
            self.clock.advance(step);
            elapsed += step;
        }
    }
}

/// Sends `payload` to `addr` and reads the echo back.
pub(super) async fn echo(addr: SocketAddr, payload: &[u8]) -> Vec<u8> {
    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    stream.write_all(payload).await.expect("write");
    let mut back = vec![0; payload.len()];
    stream.read_exact(&mut back).await.expect("read");
    back
}

mod session_backend;
