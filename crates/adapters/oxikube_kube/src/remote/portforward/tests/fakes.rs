//! A scripted cluster and an in-memory pod connector.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::channel::mpsc;
use futures::stream::BoxStream;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::{ForwardPort, ForwardSpec, ForwardStatus, OxiError, OxiResult};
use oxikube_ports::{PortForwardConnection, PortForwardPort};
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::compat::TokioAsyncReadCompatExt;

use crate::remote::portforward::cluster::Cluster;
use crate::remote::portforward::handle::ForwardHandle;
use crate::remote::portforward::plan::{
    ContainerPort, PodInfo, PodSelector, ServiceInfo, ServicePort, TargetPort,
};

/// How long a test waits for something that should already be happening.
pub(super) const DEADLINE: Duration = Duration::from_secs(5);

pub(super) fn pod(name: &str, created: i64) -> PodInfo {
    PodInfo {
        name: name.to_owned(),
        running: true,
        ready: true,
        terminating: false,
        created,
        ports: vec![
            ContainerPort {
                name: Some("http".into()),
                number: 8080,
            },
            ContainerPort {
                name: None,
                number: 9090,
            },
        ],
    }
}

pub(super) fn service(selector: &[(&str, &str)], ports: Vec<ServicePort>) -> ServiceInfo {
    ServiceInfo {
        selector: selector
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        ports,
    }
}

pub(super) fn service_port(name: Option<&str>, port: u16, target: TargetPort) -> ServicePort {
    ServicePort {
        name: name.map(str::to_owned),
        port,
        target,
    }
}

pub(super) fn spec(kind: &str, name: &str, remote: ForwardPort) -> ForwardSpec {
    ForwardSpec::new(
        ResourceRef::new(
            ClusterId::new("test", &ContextName::from("ctx")),
            Gvk::new("", "v1", kind),
            Some("default".into()),
            name,
        ),
        remote,
    )
}

/// A cluster whose pod list and watch are driven by the test.
pub(super) struct FakeCluster {
    service: Mutex<Option<ServiceInfo>>,
    pods: Mutex<Vec<PodInfo>>,
    updates: mpsc::UnboundedSender<Vec<PodInfo>>,
    watch: Mutex<Option<mpsc::UnboundedReceiver<Vec<PodInfo>>>>,
}

impl FakeCluster {
    pub(super) fn new(service: Option<ServiceInfo>, pods: Vec<PodInfo>) -> Arc<Self> {
        let (updates, watch) = mpsc::unbounded();
        Arc::new(Self {
            service: Mutex::new(service),
            pods: Mutex::new(pods),
            updates,
            watch: Mutex::new(Some(watch)),
        })
    }

    /// Makes the watch report `pods` as the current set.
    pub(super) fn push(&self, pods: Vec<PodInfo>) {
        self.updates.unbounded_send(pods).expect("watch is open");
    }
}

#[async_trait]
impl Cluster for FakeCluster {
    async fn service(&self, namespace: &str, name: &str) -> OxiResult<ServiceInfo> {
        self.service
            .lock()
            .clone()
            .ok_or_else(|| OxiError::not_found(format!("service {namespace}/{name} not found")))
    }

    async fn pods(&self, _: &str, selector: &PodSelector) -> OxiResult<Vec<PodInfo>> {
        let pods = self.pods.lock().clone();
        Ok(match selector {
            // Label matching is the API server's job; the fake serves its whole set.
            PodSelector::Labels(_) => pods,
            PodSelector::Name(name) => pods.into_iter().filter(|p| &p.name == name).collect(),
        })
    }

    fn watch_pods(&self, _: &str, _: &PodSelector) -> BoxStream<'static, Vec<PodInfo>> {
        self.watch.lock().take().expect("watched once").boxed()
    }
}

/// What the connector does with the next connection.
pub(super) enum Behaviour {
    /// Echo every byte back.
    Echo,
    /// Fail to open.
    Refuse(OxiError),
    /// Open, then report `error` on the error channel and close.
    ServerError(OxiError),
    /// Open, then close without a byte and without an error.
    CleanClose,
}

/// The pod side, in memory. Records `(namespace, pod, port)` of every call.
pub(super) struct FakeConnector {
    script: Mutex<VecDeque<Behaviour>>,
    calls: Mutex<Vec<(String, String, u16)>>,
}

impl FakeConnector {
    pub(super) fn new(script: impl IntoIterator<Item = Behaviour>) -> Arc<Self> {
        Arc::new(Self {
            script: Mutex::new(script.into_iter().collect()),
            calls: Mutex::default(),
        })
    }

    pub(super) fn calls(&self) -> Vec<(String, String, u16)> {
        self.calls.lock().clone()
    }
}

#[async_trait]
impl PortForwardPort for FakeConnector {
    async fn forward(&self, ns: &str, pod: &str, port: u16) -> OxiResult<PortForwardConnection> {
        self.calls
            .lock()
            .push((ns.to_owned(), pod.to_owned(), port));
        let behaviour = self.script.lock().pop_front().unwrap_or(Behaviour::Echo);
        let (ours, theirs) = tokio::io::duplex(64 * 1024);
        match behaviour {
            Behaviour::Refuse(err) => Err(err),
            Behaviour::Echo => {
                tokio::spawn(async move {
                    let (mut read, mut write) = tokio::io::split(theirs);
                    let _ = tokio::io::copy(&mut read, &mut write).await;
                    let _ = write.shutdown().await;
                });
                Ok(PortForwardConnection {
                    stream: Box::pin(ours.compat()),
                    closed: Box::pin(futures::future::pending()),
                })
            }
            Behaviour::CleanClose => {
                drop(theirs);
                Ok(PortForwardConnection {
                    stream: Box::pin(ours.compat()),
                    closed: Box::pin(futures::future::ready(None)),
                })
            }
            Behaviour::ServerError(err) => {
                tokio::spawn(async move {
                    // Hold the far end open briefly, then close it, like a kubelet that
                    // reports an error and hangs up.
                    let mut theirs = theirs;
                    let _ = theirs.read_u8().await;
                });
                Ok(PortForwardConnection {
                    stream: Box::pin(ours.compat()),
                    closed: Box::pin(futures::future::ready(Some(err))),
                })
            }
        }
    }
}

/// Starts `spec` on the fakes.
pub(super) async fn start(
    cluster: &Arc<FakeCluster>,
    connector: &Arc<FakeConnector>,
    spec: &ForwardSpec,
) -> OxiResult<ForwardHandle> {
    crate::remote::portforward::session::start(connector.clone(), cluster.clone(), spec).await
}

/// Waits until the handle's latest status satisfies `pred`.
pub(super) async fn wait_status(
    handle: &ForwardHandle,
    pred: impl Fn(&ForwardStatus) -> bool,
) -> ForwardStatus {
    let mut rx = handle.watch_status();
    tokio::time::timeout(DEADLINE, rx.wait_for(|s| pred(s)))
        .await
        .unwrap_or_else(|_| panic!("timed out; status is {:?}", handle.status()))
        .expect("the forward publishes until it stops")
        .clone()
}

/// Connects to the forward, sends `payload` and reads it back.
pub(super) async fn echo_through(handle: &ForwardHandle, payload: &[u8]) -> Vec<u8> {
    let mut socket = tokio::net::TcpStream::connect(handle.local_addr())
        .await
        .expect("connect to the local listener");
    socket.write_all(payload).await.expect("write");
    let mut back = vec![0; payload.len()];
    tokio::time::timeout(DEADLINE, socket.read_exact(&mut back))
        .await
        .expect("echo arrives")
        .expect("read");
    back
}
