//! Websocket-backed data plane: the streams that outlive one request (E04-S09, E04-S10).
//!
//! | Module | Story | Role |
//! |---|---|---|
//! | [`portforward`] | E04-S10 | [`PortForwardPort`](oxikube_ports::PortForwardPort) over `Api<Pod>::portforward`, plus the local-listener [`ForwardHandle`](portforward::ForwardHandle) with service-to-pod resolution and a target-gone hook |
//!
//! Exec, attach and node shells (E04-S09) join here as sibling modules.

pub mod portforward;
