//! `portforward.list_port_forwards`: the forwards the user sees in the Port Forwards panel.

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_domain::redact::redact;
use oxikube_ports::{ToolContext, ToolDef, ToolName, ToolOutput, ToolPort};
use serde_json::{Value, json};

use crate::portforward::{ForwardInfo, PortForwardManager};

/// The tool's name.
pub const LIST_PORT_FORWARDS: &str = "portforward.list_port_forwards";

/// The `list_port_forwards` tool: read-only, no confirmation.
pub struct ListPortForwardsTool {
    def: ToolDef,
    manager: PortForwardManager,
}

impl ListPortForwardsTool {
    /// The tool over `manager`.
    pub fn new(manager: PortForwardManager) -> Self {
        let def = ToolDef::read_only(
            ToolName::new(LIST_PORT_FORWARDS).expect("a well-formed name"),
            "List the port-forwards the app runs: id, cluster, target, ports, status, local URL \
             and traffic counters. Optionally only those of one cluster.",
            json!({
                "type": "object",
                "properties": {
                    "cluster": {"type": "string", "description": "Only this cluster's forwards (a cluster id)."}
                },
                "additionalProperties": false
            }),
        )
        .with_title("List Port-Forwards");
        Self { def, manager }
    }
}

impl std::fmt::Debug for ListPortForwardsTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListPortForwardsTool")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ToolPort for ListPortForwardsTool {
    fn def(&self) -> &ToolDef {
        &self.def
    }

    async fn invoke(&self, args: Value, ctx: &ToolContext) -> OxiResult<ToolOutput> {
        let only = match args.get("cluster").and_then(Value::as_str) {
            Some(id) => Some(super::cluster_id(id)?),
            None => ctx.cluster.clone(),
        };
        let rows: Vec<Value> = self
            .manager
            .list()
            .iter()
            .filter(|f| only.as_ref().is_none_or(|c| &f.spec.target.cluster == c))
            .map(row)
            .collect();
        Ok(ToolOutput::json(json!({ "forwards": rows })))
    }
}

fn row(info: &ForwardInfo) -> Value {
    let target = &info.spec.target;
    json!({
        "id": info.id.0,
        "cluster": target.cluster.to_string(),
        "target": {
            "kind": target.gvk.kind.to_string(),
            "namespace": target.namespace.as_deref(),
            "name": target.name.to_string(),
        },
        "remote_port": info.spec.remote_port.to_string(),
        "local_port": info.local_addr.map(|a| a.port()),
        "bind": info.local_addr.map_or(info.spec.bind, |a| a.ip()).to_string(),
        "status": info.state.phase(),
        "reason": info.state.reason().map(redact),
        "pod": info.pod,
        "url": info.url(),
        "traffic": {
            "bytes_in": info.stats.bytes_in,
            "bytes_out": info.stats.bytes_out,
            "active_connections": info.stats.active_connections,
            "total_connections": info.stats.total_connections,
        },
    })
}
