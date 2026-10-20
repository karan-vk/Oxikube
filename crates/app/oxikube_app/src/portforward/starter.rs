//! [`ForwardStarter`]: the start dialog's way through the bus.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use oxikube_domain::OxiError;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::portforward::ForwardSpec;

use super::manager::PortForwardManager;
use super::model::{ForwardId, StartError};
use crate::command_bus::{CommandBus, DispatchContext, DispatchError, Outcome};

/// A forward that came up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedForward {
    /// Its id in the manager.
    pub id: ForwardId,
    /// Where it listens.
    pub local_addr: SocketAddr,
    /// The pod serving it (or the Service's pod); `None` while it waits for a ready one.
    pub pod: Option<String>,
}

impl StartedForward {
    /// `http://127.0.0.1:<port>`: how to reach the forward from this machine (a wildcard bind is
    /// reached through loopback).
    pub fn url(&self) -> String {
        let host = match self.local_addr.ip() {
            ip if ip.is_unspecified() => "127.0.0.1".to_owned(),
            IpAddr::V6(ip) => format!("[{ip}]"),
            ip => ip.to_string(),
        };
        format!("http://{host}:{}", self.local_addr.port())
    }
}

/// Dispatches `portforward::Start` on the bus as the user, for a dialog that already asked. Cheap
/// to clone. The view calls [`start`](Self::start) on the Tokio bridge
/// (`oxikube_runtime::spawn_kube`).
///
/// A forward changes nothing in a cluster, so the command takes no confirmation and no guard
/// tier; its read-only policy (`portforward.allowed_in_readonly`) is the manager's, enforced in
/// the handler, and [`blocked`](Self::blocked) tells a view to disable its button up front.
#[derive(Clone)]
pub struct ForwardStarter {
    bus: CommandBus,
    who: Arc<str>,
    manager: PortForwardManager,
}

impl std::fmt::Debug for ForwardStarter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForwardStarter").finish_non_exhaustive()
    }
}

impl ForwardStarter {
    /// A starter dispatching on `bus` as `who` (the local user's name, for the log), reading the
    /// read-only policy from `manager`.
    pub fn new(bus: CommandBus, who: impl Into<Arc<str>>, manager: PortForwardManager) -> Self {
        Self {
            bus,
            who: who.into(),
            manager,
        }
    }

    /// Why a start in `cluster` would be refused now (read-only, setting off): `None` when it may
    /// start.
    pub fn blocked(&self, cluster: &ClusterId) -> Option<String> {
        self.manager.refusal(cluster).map(|e| {
            let hint = e.suggestion().map(|h| format!(" {h}")).unwrap_or_default();
            format!("{e}.{hint}")
        })
    }

    /// Starts `spec` through `portforward::Start` and waits until it is up.
    ///
    /// # Errors
    ///
    /// The handler's error, whose source is the [`StartError`] ([`StartError::of`]) so a view can
    /// offer the fix; a [`DispatchError`] mapped to the shared kinds otherwise.
    pub async fn start(&self, spec: ForwardSpec) -> Result<StartedForward, OxiError> {
        let cluster = spec.target.cluster.clone();
        let command = Command::PortForwardStart {
            target: spec.target,
            remote_port: spec.remote_port,
            local_port: (spec.local_port != 0).then_some(spec.local_port),
            bind: Some(spec.bind),
            name: spec.name,
            favourite: spec.favourite,
        };
        let context = DispatchContext::new(Initiator::Ui, self.who.clone()).with_cluster(cluster);
        let outcome = self
            .bus
            .dispatch(command, context)
            .await
            .map_err(|e| match e {
                DispatchError::Handler(inner) => inner,
                other => other.into(),
            })?;
        let Outcome::Completed(output) = outcome else {
            return Err(OxiError::internal(
                "a port-forward start asked for a confirmation",
            ));
        };
        let data = output.data.unwrap_or_default();
        let id = data
            .get("id")
            .and_then(|id| id.as_u64())
            .map(ForwardId)
            .ok_or_else(|| OxiError::internal("the start did not name the forward"))?;
        let pod = data.get("pod").and_then(|p| p.as_str()).map(str::to_owned);
        match data
            .get("local_addr")
            .and_then(|a| a.as_str())
            .and_then(|a| a.parse::<SocketAddr>().ok())
        {
            Some(local_addr) => Ok(StartedForward {
                id,
                local_addr,
                pod,
            }),
            // Still waiting for a ready pod after the start's patience: the forward is listed and
            // keeps trying, but there is no address to show yet.
            None => Err(OxiError::from(StartError::Failed(
                output
                    .message
                    .unwrap_or_else(|| "the forward did not start".to_owned()),
            ))),
        }
    }
}
