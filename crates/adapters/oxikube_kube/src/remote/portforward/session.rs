//! Starting a forward and running its task.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::stream::BoxStream;
use oxikube_domain::ids::Gvk;
use oxikube_domain::{ErrorKind, ForwardSpec, ForwardStatus, OxiError, OxiResult};
use oxikube_ports::PortForwardPort;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::bridge::Bridge;
use super::cluster::Cluster;
use super::error::bind_error;
use super::handle::ForwardHandle;
use super::hub::{StatusHub, StopGuard};
use super::monitor::{Flow, Monitor};
use super::plan::{Plan, PodInfo};

/// Pause after a failed `accept` (out of file descriptors, ...) so a persistent failure
/// cannot spin the task.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// Resolves `spec`, binds its listener and spawns the forward.
///
/// Everything that can be refused up front is refused here, before anything is spawned: an
/// unsupported target, a missing pod or service, a pod that cannot serve, a busy local port.
pub(super) async fn start(
    connector: Arc<dyn PortForwardPort>,
    cluster: Arc<dyn Cluster>,
    spec: &ForwardSpec,
) -> OxiResult<ForwardHandle> {
    let (namespace, name) = target_of(spec)?;
    let plan = match spec.target.gvk.kind.as_ref() {
        "Pod" => Plan::pod(namespace, name, spec.remote_port.clone()),
        _ => {
            let service = cluster.service(namespace, name).await?;
            Plan::service(namespace, name, service, &spec.remote_port)?
        }
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
    let bridge = Bridge {
        connector,
        namespace: namespace.into(),
        target: target_rx,
        hub: hub.clone(),
        local_addr,
    };
    let pods = cluster.watch_pods(namespace, &selector);
    let task = tokio::spawn(run(listener, pods, monitor, bridge, StopGuard(hub.clone())));
    Ok(ForwardHandle::new(local_addr, hub, task))
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

/// The forward's task. Owns the listener and every bridge: when it ends or is aborted they
/// all go with it.
async fn run(
    listener: TcpListener,
    mut pods: BoxStream<'static, Vec<PodInfo>>,
    mut monitor: Monitor,
    bridge: Bridge,
    stop: StopGuard,
) {
    let mut bridges = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((local, _peer)) => {
                    bridges.spawn(bridge.clone().serve(local));
                }
                Err(err) => {
                    tracing::warn!(error = %err, "port-forward accept failed");
                    stop.0.publish(ForwardStatus::Error {
                        kind: ErrorKind::Internal,
                        message: format!("accepting a local connection failed: {err}"),
                    });
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                }
            },
            snapshot = pods.next() => match snapshot {
                Some(pods) => {
                    if monitor.on_snapshot(&pods) == Flow::Stop {
                        break;
                    }
                }
                None => {
                    stop.0.publish(ForwardStatus::Error {
                        kind: ErrorKind::Internal,
                        message: "the pod watch ended unexpectedly".to_owned(),
                    });
                    break;
                }
            },
            // Reap finished bridges so the set does not grow with every connection.
            Some(_) = bridges.join_next(), if !bridges.is_empty() => {}
        }
    }
    // `stop` publishes `Stopped` as it drops, after the listener and bridges are gone.
    drop(listener);
    bridges.shutdown().await;
    drop(stop);
}
