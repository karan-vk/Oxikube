//! `PortForwardPort::start` for [`KubePortForward`]: a pod forward behind the port-level
//! [`ForwardHandle`].

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use futures::channel::mpsc;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::{ForwardPort, ForwardSpec, ForwardStatus};
use oxikube_ports::{ForwardHandle, ForwardStats, PodRef, PortForwardError};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

use super::connect::KubePortForward;
use super::error::to_port_error as classify;
use super::handle::ForwardHandle as ListenerHandle;

/// Aborts the task on drop; the task owns the listener, so the port is freed with it.
struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(super) async fn start(
    adapter: &KubePortForward,
    target: PodRef,
    remote_port: u16,
    bind: IpAddr,
    local_port: Option<u16>,
) -> Result<ForwardHandle, PortForwardError> {
    let requested = SocketAddr::new(bind, local_port.unwrap_or(0));
    let spec = ForwardSpec::new(
        // The adapter serves one cluster; the id is only a label here.
        ResourceRef::new(
            ClusterId::new("kubeconfig", &ContextName::from("local")),
            Gvk::new("", "v1", "Pod"),
            Some(target.namespace.as_str().into()),
            target.name.as_str(),
        ),
        ForwardPort::Number(remote_port),
    )
    .with_local_port(requested.port())
    .with_bind(bind);

    let stats = Arc::new(ForwardStats::default());
    let listener = adapter
        .start_with_stats(&spec, stats.clone())
        .await
        .map_err(|err| classify(&err, &target, requested, remote_port))?;
    let local_addr = listener.local_addr();
    tracing::debug!(pod = %target, remote_port, %local_addr, "port forward started");
    let (errors_tx, errors) = mpsc::unbounded();
    let graceful = listener.graceful_stop();
    let task = tokio::spawn(report_errors(
        listener,
        target,
        requested,
        remote_port,
        errors_tx,
    ));
    Ok(
        ForwardHandle::new(local_addr, stats, errors, AbortOnDrop(task))
            .with_graceful_stop(move || Box::pin(graceful)),
    )
}

/// Turns the listener's status transitions into [`PortForwardError`]s until the forward ends.
async fn report_errors(
    listener: ListenerHandle,
    pod: PodRef,
    requested: SocketAddr,
    remote_port: u16,
    errors: mpsc::UnboundedSender<PortForwardError>,
) {
    let mut events = listener.events();
    loop {
        let error = match events.recv().await {
            Ok(ForwardStatus::Error { kind, message }) => {
                let err = oxikube_domain::OxiError::new(kind, message);
                classify(&err, &pod, requested, remote_port)
            }
            Ok(ForwardStatus::TargetGone { pod: gone }) => {
                PortForwardError::PodGone(format!("{}/{gone}", pod.namespace))
            }
            Ok(ForwardStatus::Stopped) | Err(RecvError::Closed) => break,
            Ok(_) | Err(RecvError::Lagged(_)) => continue,
        };
        if errors.unbounded_send(error).is_err() {
            break;
        }
    }
    // `listener` drops here: a pod forward that ended on its own frees its port.
}
