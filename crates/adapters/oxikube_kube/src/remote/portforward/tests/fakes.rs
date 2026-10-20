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
use oxikube_ports::{PodRef, PortForwardConnection, PortForwardError, PortForwardPort};
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
    /// Echo every chunk back after this delay (a pod still working on its reply).
    SlowEcho(Duration),
    /// Fail to open.
    Refuse(OxiError),
    /// Open, then report `error` on the error channel and close.
    ServerError(OxiError),
    /// Open, then close without a byte and without an error.
    CleanClose,
    /// Open, send these bytes, then close cleanly (the pod's process hung up mid-session).
    SendThenClose(Vec<u8>),
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
    async fn start(
        &self,
        _: PodRef,
        _: u16,
        _: std::net::IpAddr,
        _: Option<u16>,
    ) -> Result<oxikube_ports::ForwardHandle, PortForwardError> {
        self.unsupported_start().await
    }

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
            Behaviour::SlowEcho(delay) => {
                tokio::spawn(async move {
                    let (mut read, mut write) = tokio::io::split(theirs);
                    let mut chunk = vec![0u8; 16 * 1024];
                    while let Ok(n @ 1..) = read.read(&mut chunk).await {
                        tokio::time::sleep(delay).await;
                        if write.write_all(&chunk[..n]).await.is_err() {
                            break;
                        }
                    }
                    let _ = write.shutdown().await;
                });
                Ok(PortForwardConnection {
                    stream: Box::pin(ours.compat()),
                    closed: Box::pin(futures::future::pending()),
                })
            }
            Behaviour::SendThenClose(bytes) => {
                tokio::spawn(async move {
                    let mut theirs = theirs;
                    let _ = theirs.write_all(&bytes).await;
                    let _ = theirs.shutdown().await;
                });
                Ok(PortForwardConnection {
                    stream: Box::pin(ours.compat()),
                    closed: Box::pin(futures::future::ready(None)),
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

impl FakeConnector {
    /// The connector only serves `forward`; `start` is the adapter's own job.
    async fn unsupported_start(&self) -> Result<oxikube_ports::ForwardHandle, PortForwardError> {
        Err(PortForwardError::Other(
            "FakeConnector does not start forwards".into(),
        ))
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

/// Starts `spec` on the fakes serving at most `max_connections` at once.
pub(super) async fn start_capped(
    cluster: &Arc<FakeCluster>,
    connector: &Arc<FakeConnector>,
    spec: &ForwardSpec,
    max_connections: usize,
) -> OxiResult<ForwardHandle> {
    use crate::remote::portforward::session::{Options, start_with};
    let options = Options {
        max_connections,
        ..Options::default()
    };
    start_with(connector.clone(), cluster.clone(), spec, options).await
}

/// Waits until `cond` holds, yielding in between; panics after [`DEADLINE`].
pub(super) async fn eventually(what: &str, cond: impl Fn() -> bool) {
    tokio::time::timeout(DEADLINE, async {
        while !cond() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
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
