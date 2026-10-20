//! `portforward.start_port_forward`: start a forward on an agent's behalf, behind the policy of
//! the UI and an explicit permission.
//!
//! | Argument | Meaning |
//! |---|---|
//! | `cluster` | the cluster id; the call's own cluster when omitted |
//! | `kind` | `pod` or `service` (required) |
//! | `name`, `namespace` | the target (`namespace` default `default`) |
//! | `remote_port` | the target's port (required) |
//! | `local_port` | the local port; unset picks a free one (from `portforward.port_range` when set) |
//! | `bind` | the listen address; loopback, or the user's `portforward.default_bind` |

use std::net::IpAddr;
use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::portforward::{ForwardPort, ForwardSpec, PortRange};
use oxikube_domain::redact::redact;
use oxikube_domain::{Capabilities, OxiError, OxiResult};
use oxikube_ports::{ToolContext, ToolDef, ToolName, ToolOutput, ToolPort};
use serde_json::{Value, json};

use super::permission::{ForwardPermission, StartRequest};
use crate::portforward::{ForwardConfig, ForwardInfo, PortForwardManager, StartError};

/// The tool's name.
pub const START_PORT_FORWARD: &str = "portforward.start_port_forward";

/// The `start_port_forward` tool: gated, see the [module docs](super).
pub struct StartPortForwardTool {
    def: ToolDef,
    manager: PortForwardManager,
    permission: Arc<dyn ForwardPermission>,
}

impl StartPortForwardTool {
    /// The tool over `manager`, asking `permission` before any listener opens.
    pub fn new(manager: PortForwardManager, permission: Arc<dyn ForwardPermission>) -> Self {
        let def = ToolDef::read_only(
            ToolName::new(START_PORT_FORWARD).expect("a well-formed name"),
            "Start a port-forward to a pod or Service, opening a local listener. Needs the \
             user's permission; refused on a read-only cluster unless the user allowed \
             forwards there, listens on loopback unless the user set another default address, \
             and keeps a requested local port inside the user's port range.",
            json!({
                "type": "object",
                "properties": {
                    "cluster": {"type": "string", "description": "Cluster id; the current cluster when omitted."},
                    "kind": {"enum": ["pod", "service"]},
                    "name": {"type": "string", "minLength": 1},
                    "namespace": {"type": "string", "minLength": 1, "description": "Default `default`."},
                    "remote_port": {"type": "integer", "minimum": 1, "maximum": 65535},
                    "local_port": {"type": "integer", "minimum": 1, "maximum": 65535, "description": "Unset picks a free port."},
                    "bind": {"type": "string", "description": "Listen address: loopback, or the user's default address."}
                },
                "required": ["kind", "name", "remote_port"],
                "additionalProperties": false
            }),
        )
        .with_title("Start Port-Forward")
        .with_needs(Capabilities::PORTFORWARD);
        Self {
            def,
            manager,
            permission,
        }
    }
}

impl std::fmt::Debug for StartPortForwardTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartPortForwardTool")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ToolPort for StartPortForwardTool {
    fn def(&self) -> &ToolDef {
        &self.def
    }

    async fn invoke(&self, args: Value, ctx: &ToolContext) -> OxiResult<ToolOutput> {
        let call = Call::parse(&args, ctx)?;
        let config = self.manager.config_for(&call.target.cluster);
        let bind = match call.allowed_bind(&config) {
            Ok(bind) => bind,
            Err(reason) => return Ok(ToolOutput::error(reason)),
        };
        if let Err(reason) = check_local_port(call.local_port, config.port_range) {
            return Ok(ToolOutput::error(reason));
        }
        if let Some(refusal) = self.manager.refusal(&call.target.cluster) {
            return Ok(failed(refusal));
        }
        let request = StartRequest {
            target: call.target.clone(),
            remote_port: call.remote_port,
            local_port: call.local_port,
            bind,
        };
        if !self.permission.allows(ctx, &request).await {
            return Ok(ToolOutput::error(
                "starting a port-forward needs the user's permission, which was not given",
            ));
        }
        // An audit line, not a cluster mutation: who opened a local listener to what.
        tracing::info!(
            initiator = "agent",
            cluster = %call.target.cluster,
            namespace = call.target.namespace.as_deref().unwrap_or_default(),
            target = %call.target.name,
            remote_port = call.remote_port,
            "agent started a port-forward"
        );
        let mut spec = ForwardSpec::new(call.target, ForwardPort::Number(call.remote_port));
        spec.local_port = call.local_port.unwrap_or(0);
        spec.bind = bind;
        match self.manager.start_and_wait(spec).await {
            Ok(info) => Ok(started(&info)),
            Err(err) => Ok(failed(err)),
        }
    }
}

/// The arguments of one call.
struct Call {
    target: ResourceRef,
    remote_port: u16,
    local_port: Option<u16>,
    bind: Option<IpAddr>,
}

impl Call {
    fn parse(args: &Value, ctx: &ToolContext) -> OxiResult<Self> {
        let text = |key: &str| args.get(key).and_then(Value::as_str);
        let port = |key: &str| {
            args.get(key)
                .and_then(Value::as_u64)
                .map(|n| {
                    u16::try_from(n)
                        .map_err(|_| OxiError::validation(format!("{key} is not a port")))
                })
                .transpose()
        };
        let cluster = match text("cluster") {
            Some(id) => super::cluster_id(id)?,
            None => ctx.cluster.clone().ok_or_else(|| {
                OxiError::validation("name the `cluster`: the call has no current cluster")
            })?,
        };
        let kind = match text("kind") {
            Some("pod") => "Pod",
            Some("service") => "Service",
            _ => return Err(OxiError::validation("`kind` is `pod` or `service`")),
        };
        let name = text("name").ok_or_else(|| OxiError::validation("`name` is required"))?;
        let namespace = text("namespace").unwrap_or("default");
        let bind = text("bind")
            .map(|b| {
                b.parse::<IpAddr>()
                    .map_err(|_| OxiError::validation(format!("`bind` {b:?} is not an IP address")))
            })
            .transpose()?;
        Ok(Self {
            target: ResourceRef::new(
                cluster,
                Gvk::new("", "v1", kind),
                Some(namespace.into()),
                name,
            ),
            remote_port: port("remote_port")?
                .ok_or_else(|| OxiError::validation("`remote_port` is required"))?,
            local_port: port("local_port")?,
            bind,
        })
    }

    /// The address to listen on: the user's default, or the one asked for when it is loopback
    /// or exactly that default. Anything else would expose the port to the network without the
    /// user having said so.
    fn allowed_bind(&self, config: &ForwardConfig) -> Result<IpAddr, String> {
        match self.bind {
            None => Ok(config.default_bind),
            Some(bind) if bind.is_loopback() || bind == config.default_bind => Ok(bind),
            Some(bind) => Err(format!(
                "listening on {bind} is not allowed: agents may listen on loopback or on \
                 portforward.default_bind ({}); the user can change that setting",
                config.default_bind
            )),
        }
    }
}

fn check_local_port(port: Option<u16>, range: Option<PortRange>) -> Result<(), String> {
    let Some(port) = port else { return Ok(()) };
    if port < PortRange::MIN_START {
        return Err(format!(
            "local port {port} is privileged: use {} or higher, or leave it unset",
            PortRange::MIN_START
        ));
    }
    match range {
        Some(range) if !range.contains(port) => Err(format!(
            "local port {port} is outside portforward.port_range ({range}): pick one inside it, \
             or leave it unset"
        )),
        _ => Ok(()),
    }
}

fn started(info: &ForwardInfo) -> ToolOutput {
    ToolOutput::json(json!({
        "id": info.id.0,
        "status": info.state.phase(),
        "local_port": info.local_addr.map(|a| a.port()),
        "url": info.url(),
        "pod": info.pod,
    }))
}

fn failed(err: StartError) -> ToolOutput {
    let err = OxiError::from(err);
    ToolOutput::error(format!("{}: {}", err.kind(), redact(err.message())))
}
