//! `portforward::Start`, `Stop`, `Restart` and `Remove` on the command bus (each with its MCP
//! tool stub), `pod::PortForward` (a start on a pod port, the row action the start dialog stands
//! in front of), and the manager as the [`ServicePortOpener`] behind `service::OpenInBrowser`.
//!
//! None changes a cluster, so none goes through a guard tier; the read-only policy is the
//! manager's (`portforward.allowed_in_readonly`).

use async_trait::async_trait;
use futures::StreamExt as _;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::portforward::{ForwardPort, ForwardSpec};
use oxikube_domain::session::WatchScope;
use oxikube_domain::view::ServiceSummary;
use oxikube_domain::{OxiError, OxiResult};
use serde_json::json;

use super::manager::PortForwardManager;
use super::model::{ForwardId, ForwardInfo, StartError};
use super::state::ForwardState;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use crate::network::{OpenedServicePort, ServicePortOpener};
use crate::store::{StoreFilter, StoreQuery};

/// The commands [`register_commands`] installs.
pub const COMMANDS: [CommandId; 5] = [
    CommandId::POD_PORT_FORWARD,
    CommandId::PORTFORWARD_START,
    CommandId::PORTFORWARD_STOP,
    CommandId::PORTFORWARD_RESTART,
    CommandId::PORTFORWARD_REMOVE,
];

/// Registers the manager's commands on `registry`. The binary installs it as
/// `oxikube_app::portforward`.
///
/// # Errors
///
/// A [`RegisterError`] when one of the ids is already taken.
pub fn register_commands(
    registry: &mut CommandRegistry,
    manager: PortForwardManager,
) -> Result<(), RegisterError> {
    for id in COMMANDS {
        let manager = manager.clone();
        let meta = *command::lookup(id).expect("declared in oxikube_domain::command");
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let manager = manager.clone();
            async move { run(&manager, command).await }
        })?;
    }
    Ok(())
}

pub(super) async fn run(
    manager: &PortForwardManager,
    command: Command,
) -> OxiResult<CommandOutput> {
    match command {
        Command::PortForwardStart {
            target,
            remote_port,
            local_port,
            bind,
            name,
            favourite,
        } => {
            if remote_port == ForwardPort::Number(0) {
                return Err(OxiError::validation("choose the port to forward"));
            }
            let mut spec = ForwardSpec::new(target, remote_port).with_favourite(favourite);
            spec.local_port = local_port.unwrap_or(0);
            spec.name = name.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty());
            spec.bind =
                bind.unwrap_or_else(|| manager.config_for(&spec.target.cluster).default_bind);
            let saved = favourite.then(|| spec.clone());
            let info = manager.start_and_wait(spec).await?;
            let mut out = output(&info);
            // "Keep as a favourite": saved once the forward is accepted. A save that fails does
            // not undo the forward; the output says why so the caller can tell the user.
            if let Some(spec) = saved
                && let Err(error) = manager.save_favourite(spec).await
            {
                let error = error.to_string();
                if let Some(message) = out.message.as_mut() {
                    message.push_str(&format!(" (not saved as a favourite: {error})"));
                }
                if let Some(serde_json::Value::Object(data)) = out.data.as_mut() {
                    data.insert("favourite_error".into(), json!(error));
                }
            }
            Ok(out)
        }
        Command::PodPortForward {
            target,
            local_port,
            remote_port,
        } => {
            let start = Command::PortForwardStart {
                target,
                remote_port: ForwardPort::Number(remote_port),
                local_port,
                bind: None,
                name: None,
                favourite: false,
            };
            Box::pin(run(manager, start)).await
        }
        Command::PortForwardStop { id } => {
            known(manager.stop(ForwardId(id)), id)?;
            Ok(done(id, "stopped"))
        }
        Command::PortForwardRestart { id } => {
            manager.restart(ForwardId(id))?;
            Ok(done(id, "restarted"))
        }
        Command::PortForwardRemove { id } => {
            known(manager.remove(ForwardId(id)), id)?;
            Ok(done(id, "removed"))
        }
        other => Err(OxiError::internal(format!(
            "{} is not a port-forward command",
            other.id()
        ))),
    }
}

fn known(found: bool, id: u64) -> OxiResult<()> {
    found
        .then_some(())
        .ok_or_else(|| StartError::Unknown(ForwardId(id)).into())
}

fn done(id: u64, what: &str) -> CommandOutput {
    CommandOutput {
        message: Some(format!("Port-forward {id} {what}")),
        data: Some(json!({ "id": id })),
    }
}

fn output(info: &ForwardInfo) -> CommandOutput {
    let message = match (&info.state, info.local_addr) {
        (ForwardState::Active { pod, local_addr }, _) => {
            format!("Forwarding {local_addr} to pod {pod}")
        }
        (state, _) => format!(
            "Port-forward {}: {}",
            info.label(),
            state.reason().unwrap_or("starting")
        ),
    };
    CommandOutput {
        message: Some(message),
        data: Some(json!({
            "id": info.id.0,
            "local_addr": info.local_addr.map(|a| a.to_string()),
            "pod": info.pod,
            "phase": info.state.phase(),
        })),
    }
}

#[async_trait]
impl ServicePortOpener for PortForwardManager {
    /// Reuses an active forward of the same Service port, else starts one and waits for it.
    async fn open_service_port(
        &self,
        service: ResourceRef,
        port: Option<u16>,
    ) -> Result<OpenedServicePort, OxiError> {
        let summary = self.read_service(&service).await?;
        let service_port = summary
            .ports
            .iter()
            .find(|p| port.map_or(p.is_tcp(), |n| p.port == n))
            .ok_or_else(|| {
                OxiError::validation(match port {
                    Some(n) => format!("Service {} has no port {n}", summary.name),
                    None => format!("Service {} has no TCP port", summary.name),
                })
            })?
            .clone();
        let remote_port = ForwardPort::Number(service_port.port);
        let active = self.list().into_iter().find(|f| {
            f.spec.target == service
                && f.spec.remote_port == remote_port
                && matches!(f.state, ForwardState::Active { .. })
        });
        let info = match active {
            Some(info) => info,
            None => {
                let bind = self.config_for(&service.cluster).default_bind;
                self.start_and_wait(ForwardSpec::new(service, remote_port).with_bind(bind))
                    .await?
            }
        };
        let local = match (&info.state, info.local_addr) {
            (ForwardState::Active { .. }, Some(addr)) => addr.port(),
            (state, _) => {
                return Err(OxiError::conflict(format!(
                    "The forward is not up yet: {}",
                    state.reason().unwrap_or("starting")
                )));
            }
        };
        Ok(OpenedServicePort {
            url: service_port.browser_url(local),
            service_port: service_port.port,
        })
    }
}

impl PortForwardManager {
    /// The Service `target`, read from the store once its feed has listed.
    async fn read_service(&self, target: &ResourceRef) -> Result<ServiceSummary, OxiError> {
        let namespace = target.namespace.as_deref().unwrap_or_default().to_owned();
        let query = StoreQuery::new(target.gvk.clone(), WatchScope::Namespaces(vec![namespace]))
            .with_filter(StoreFilter {
                name: Some(target.name.to_string()),
                ..StoreFilter::default()
            });
        let mut watch = self
            .backend()
            .watch(&target.cluster, query)
            .ok_or_else(|| StartError::NotConnected(target.cluster.to_string()))?;
        let objects = watch
            .next()
            .await
            .ok_or_else(|| OxiError::not_found("the cluster disconnected"))?
            .map_err(OxiError::forbidden)?;
        let service = objects
            .first()
            .ok_or_else(|| OxiError::not_found(format!("no Service {}", target.name)))?;
        ServiceSummary::from_resource(service).map_err(|err| OxiError::validation(err.to_string()))
    }
}
