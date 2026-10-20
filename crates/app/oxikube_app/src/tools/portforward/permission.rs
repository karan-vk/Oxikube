//! The agent-permission seam for `start_port_forward`.

use std::net::IpAddr;

use async_trait::async_trait;
use oxikube_domain::ids::ResourceRef;
use oxikube_ports::ToolContext;

/// What an agent asks to forward: shown to the user by the permission prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRequest {
    /// The pod or Service.
    pub target: ResourceRef,
    /// The port on the target.
    pub remote_port: u16,
    /// The local port asked for; `None` lets the manager pick one.
    pub local_port: Option<u16>,
    /// The address the forward would listen on.
    pub bind: IpAddr,
}

/// Who may start a forward on an agent's behalf. The agent epic's permission prompt (E27)
/// implements it by asking the user; until then the app installs [`DenyForwards`].
#[async_trait]
pub trait ForwardPermission: Send + Sync {
    /// Whether the caller described by `ctx` may start the forward in `request`.
    async fn allows(&self, ctx: &ToolContext, request: &StartRequest) -> bool;
}

/// The default policy: nobody. A call that asks is a tool error the model sees.
#[derive(Debug, Clone, Copy, Default)]
pub struct DenyForwards;

#[async_trait]
impl ForwardPermission for DenyForwards {
    async fn allows(&self, _: &ToolContext, _: &StartRequest) -> bool {
        false
    }
}
