//! [`KubePortForward`]: `PortForwardPort` on kube's `Portforwarder`, and the entry point for
//! local-listener forwards.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Pod;
use kube::api::Portforwarder;
use kube::{Api, Client};
use oxikube_domain::{ForwardSpec, OxiError, OxiResult};
use oxikube_ports::{
    ForwardStats, PodRef, PortForwardConnection, PortForwardError, PortForwardPort,
};
use std::net::IpAddr;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_util::compat::TokioAsyncReadCompatExt;

use super::accept::DEFAULT_MAX_CONNECTIONS;
use super::cluster::{Cluster, KubeCluster};
use super::error::{open_error, remote_failure};
use super::handle::ForwardHandle;
use super::session::{self, Options};
use super::start;

/// Port forwarding on one connected cluster. Cheap to clone; clones share the client.
///
/// Two levels, one adapter:
///
/// * [`PortForwardPort::forward`] opens one byte stream to one pod port (a `Portforwarder`
///   per call). Dropping the stream aborts the forwarder.
/// * [`start`](Self::start) runs a whole forward: a local TCP listener, service-to-pod
///   resolution, the restart hook and a status feed (see [`ForwardHandle`]).
///
/// A port forward is not a cluster mutation, so it does not go through `MutationGuard`;
/// callers gate it on the session's port-forward capability.
#[derive(Clone)]
pub struct KubePortForward {
    client: Client,
    cluster: Arc<dyn Cluster>,
    max_connections: usize,
}

impl KubePortForward {
    /// Forwards through `client`.
    pub fn new(client: Client) -> Self {
        Self {
            cluster: Arc::new(KubeCluster::new(client.clone())),
            client,
            max_connections: DEFAULT_MAX_CONNECTIONS,
        }
    }

    /// Caps the connections each forward serves at once (default
    /// [`DEFAULT_MAX_CONNECTIONS`]). One more is closed immediately and reported as a
    /// `BudgetExceeded` error on the forward. `0` is treated as `1`.
    #[must_use]
    pub fn with_max_connections(mut self, max: usize) -> Self {
        self.max_connections = max.max(1);
        self
    }

    fn options(&self, stats: Arc<ForwardStats>) -> Options {
        Options {
            stats,
            max_connections: self.max_connections,
        }
    }

    /// Starts forwarding `spec`: resolves the target, binds the listener and returns the
    /// handle that owns the forward. `spec.target.cluster` is not checked: this adapter
    /// serves the one cluster its client points at.
    ///
    /// # Errors
    ///
    /// Refused before anything is spawned: `Validation` for a target that is not a namespaced
    /// Pod or Service, a service without a selector or without the requested port, or a pod
    /// without the requested named port; `NotFound` for a missing pod or service (and a
    /// service with no ready pod, retryable); `Conflict` for a pod that is not running or is
    /// terminating, and for a local port that is already in use (the message names it);
    /// plus the usual transport kinds for the lookups.
    pub async fn start(&self, spec: &ForwardSpec) -> OxiResult<ForwardHandle> {
        let options = self.options(Arc::default());
        session::start_with(Arc::new(self.clone()), self.cluster.clone(), spec, options).await
    }

    /// [`start`](Self::start) counting traffic into `stats`.
    pub(super) async fn start_with_stats(
        &self,
        spec: &ForwardSpec,
        stats: Arc<ForwardStats>,
    ) -> OxiResult<ForwardHandle> {
        let options = self.options(stats);
        session::start_with(Arc::new(self.clone()), self.cluster.clone(), spec, options).await
    }
}

impl std::fmt::Debug for KubePortForward {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KubePortForward").finish_non_exhaustive()
    }
}

#[async_trait]
impl PortForwardPort for KubePortForward {
    async fn forward(
        &self,
        namespace: &str,
        pod: &str,
        port: u16,
    ) -> OxiResult<PortForwardConnection> {
        let api: Api<Pod> = Api::namespaced(self.client.clone(), namespace);
        let mut forwarder = api
            .portforward(pod, &[port])
            .await
            .map_err(|err| open_error(&err, namespace, pod, port))?;
        let (Some(stream), Some(errors)) =
            (forwarder.take_stream(port), forwarder.take_error(port))
        else {
            return Err(OxiError::internal("the port forwarder returned no stream"));
        };
        let stream = AbortOnDrop { stream, forwarder };
        Ok(PortForwardConnection {
            stream: Box::pin(stream.compat()),
            closed: Box::pin(async move { errors.await.map(|message| remote_failure(&message)) }),
        })
    }

    /// A pod forward on a local listener; see [`start`](Self::start) for the richer
    /// [`ForwardSpec`] form (services, named ports).
    async fn start(
        &self,
        target: PodRef,
        remote_port: u16,
        bind: IpAddr,
        local_port: Option<u16>,
    ) -> Result<oxikube_ports::ForwardHandle, PortForwardError> {
        start::start(self, target, remote_port, bind, local_port).await
    }
}

/// A forwarded port's stream that owns its [`Portforwarder`]: dropping the stream aborts the
/// forwarder's background task (kube's own drop only detaches it).
struct AbortOnDrop<S> {
    stream: S,
    forwarder: Portforwarder,
}

impl<S> Drop for AbortOnDrop<S> {
    fn drop(&mut self) {
        self.forwarder.abort();
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for AbortOnDrop<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for AbortOnDrop<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
