//! Port-forward tools for hosted agents (E15-S08): `portforward.list_port_forwards` and
//! `portforward.start_port_forward`, both over the app's [`PortForwardManager`].
//!
//! | Piece | Where |
//! |---|---|
//! | the read-only list: id, cluster, target, ports, status, URL, traffic | [`ListPortForwardsTool`] |
//! | the gated start: policy checks, then the agent-permission prompt, then the manager | [`StartPortForwardTool`] |
//! | who may start a forward on an agent's behalf (default: nobody until E27's prompt) | [`ForwardPermission`], [`DenyForwards`] |
//! | registering both | [`install`] |
//!
//! A forward changes nothing in the cluster, so neither tool is a mutation (no `MutationGuard`
//! tier, no [`risk`](oxikube_ports::ToolDef::risk)); but starting one opens a local listener, a
//! privileged capability. `start_port_forward` therefore needs the `portforward` capability, is
//! refused unless the permission policy allows it, follows the same read-only rule as the UI
//! (`portforward.allowed_in_readonly`), listens on loopback or the user's `portforward.default_bind`
//! only, and keeps a requested local port inside `portforward.port_range`. Every agent-started
//! forward is logged with initiator `agent`. Nothing the forwarded connections carry is ever read
//! or logged.
//!
//! The user-facing commands (`portforward::Start` and the rest) are registered by
//! [`register_commands`](crate::portforward::register_commands); the bus gives each its own tool
//! stub. These two tools are the agent's typed view of the same manager.
//!
//! [`PortForwardManager`]: crate::PortForwardManager

mod install;
mod list;
mod permission;
mod start;

pub use install::install;
pub use list::{LIST_PORT_FORWARDS, ListPortForwardsTool};
pub use permission::{DenyForwards, ForwardPermission, StartRequest};
pub use start::{START_PORT_FORWARD, StartPortForwardTool};

/// A cluster id argument, or a validation error naming it.
fn cluster_id(text: &str) -> oxikube_domain::OxiResult<oxikube_domain::ids::ClusterId> {
    text.parse().map_err(|_| {
        oxikube_domain::OxiError::validation(format!("`cluster` {text:?} is not a cluster id"))
    })
}
