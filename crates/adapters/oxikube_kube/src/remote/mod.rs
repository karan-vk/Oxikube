//! Websocket-backed data plane: the streams that outlive one request (E04-S09, E04-S10).
//!
//! | Module | Story | Role |
//! |---|---|---|
//! | [`exec`] | E04-S09 | [`ExecPort`](oxikube_ports::ExecPort) over `Api<Pod>::exec` / `attach` ([`KubeExec`](exec::KubeExec)), plus node shells (privileged pod, cleanup guaranteed) and ephemeral debug containers |
//! | [`portforward`] | E04-S10 | [`PortForwardPort`](oxikube_ports::PortForwardPort) over `Api<Pod>::portforward`, plus the local-listener [`ForwardHandle`](portforward::ForwardHandle) with service-to-pod resolution and a target-gone hook |

pub mod exec;
pub mod portforward;
