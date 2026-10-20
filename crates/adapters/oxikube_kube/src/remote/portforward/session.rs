//! Starting a forward and running its task.

use std::net::SocketAddr;
use std::sync::Arc;

use oxikube_domain::ids::Gvk;
use oxikube_domain::{ForwardSpec, OxiError, OxiResult};
use oxikube_ports::{ForwardStats, PortForwardPort};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::accept::{Accept, DEFAULT_MAX_CONNECTIONS};
use super::bridge::Bridge;
use super::cluster::Cluster;
use super::error::bind_error;
use super::handle::ForwardHandle;
use super::hub::{StatusHub, StopGuard};
use super::monitor::Monitor;
use super::plan::Plan;

/// Per-forward knobs that are not part of the [`ForwardSpec`].
#[derive(Debug, Clone)]
pub(super) struct Options {
    /// Traffic counters to fill (shared with the port-level handle).
    pub(super) stats: Arc<ForwardStats>,
    /// Concurrent connections served before new ones are refused.
    pub(super) max_connections: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            stats: Arc::default(),
            max_connections: DEFAULT_MAX_CONNECTIONS,
        }
    }
}

/// Resolves `spec`, binds its listener and spawns the forward.
///
/// Everything that can be refused up front is refused here, before anything is spawned: an
/// unsupported target, a missing pod or service, a pod that cannot serve, a busy local port.
#[cfg(test)]
pub(super) async fn start(
    connector: Arc<dyn PortForwardPort>,
    cluster: Arc<dyn Cluster>,
    spec: &ForwardSpec,
) -> OxiResult<ForwardHandle> {
    start_with(connector, cluster, spec, Options::default()).await
}

/// [`start`] with explicit [`Options`].
pub(super) async fn start_with(
    connector: Arc<dyn PortForwardPort>,
    cluster: Arc<dyn Cluster>,
    spec: &ForwardSpec,
    options: Options,
) -> OxiResult<ForwardHandle> {
    let Options {
        stats,
        max_connections,
    } = options;
    let (namespace, name) = target_of(spec)?;
    let plan = if spec.targets_service() {
        let service = cluster.service(namespace, name).await?;
        Plan::service(namespace, name, service, &spec.remote_port)?
    } else {
        Plan::pod(namespace, name, spec.remote_port.clone())
    };
    let selector = plan.selector();
    let pods = cluster.pods(namespace, &selector).await?;
    let initial = plan.pick(&pods).ok_or_else(|| plan.why_no_target(&pods))?;

    let requested = SocketAddr::new(spec.bind, spec.local_port);
    let listener = TcpListener::bind(requested)
        .await
        .map_err(|err| bind_error(requested, &err))?;
    let local_addr = listener
        .local_addr()
        .map_err(|err| OxiError::internal(format!("could not read the bound address: {err}")))?;
    if !local_addr.ip().is_loopback() {
        tracing::warn!(%local_addr, "port-forward listener is reachable beyond this machine");
    }

    let hub = StatusHub::new();
    let (target_tx, target_rx) = watch::channel(None);
    // Publishes `Listening` and the first target before the task exists.
    let monitor = Monitor::start(plan, hub.clone(), target_tx, local_addr, initial);
    let shutdown = CancellationToken::new();
    let bridge = Bridge {
        connector,
        namespace: namespace.into(),
        target: target_rx,
        hub: hub.clone(),
        local_addr,
        stats: stats.clone(),
        shutdown: shutdown.clone(),
    };
    let accept = Accept {
        listener,
        pods: cluster.watch_pods(namespace, &selector),
        monitor,
        bridge,
        stop: StopGuard(hub.clone()),
        shutdown: shutdown.clone(),
        max_connections,
    };
    let task = tokio::spawn(accept.run());
    Ok(ForwardHandle::new(local_addr, hub, stats, shutdown, task))
}

/// The namespace and name of a Pod or Service target.
fn target_of(spec: &ForwardSpec) -> OxiResult<(&str, &str)> {
    let gvk = &spec.target.gvk;
    if *gvk != Gvk::new("", "v1", "Pod") && *gvk != Gvk::new("", "v1", "Service") {
        return Err(OxiError::validation(format!(
            "port-forward targets a Pod or a Service, not {gvk}"
        )));
    }
    let name: &str = &spec.target.name;
    match spec.target.namespace.as_deref() {
        Some(namespace) if !namespace.is_empty() => Ok((namespace, name)),
        _ => Err(OxiError::validation(format!(
            "port-forward target {name} needs a namespace"
        ))),
    }
}
