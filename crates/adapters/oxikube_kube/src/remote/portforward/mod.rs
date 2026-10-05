//! Port forwarding on kube-rs (E04-S10).
//!
//! [`KubePortForward`] is the adapter for one cluster. It does two jobs:
//!
//! * [`PortForwardPort::forward`](oxikube_ports::PortForwardPort::forward): one byte stream
//!   to one pod port, from `Api<Pod>::portforward` (`take_stream` bridged to `futures::io`
//!   with `tokio_util::compat`, `take_error` surfaced as the connection's `closed` future).
//! * [`KubePortForward::start`]: a whole forward for a [`ForwardSpec`]: a local TCP listener
//!   (loopback by default, port `0` picks a free one), service-to-pod resolution, a
//!   restart hook and a status feed, owned by a [`ForwardHandle`].
//!
//! | Piece | Where |
//! |---|---|
//! | `PortForwardPort` impl, abort-on-drop stream | `connect` |
//! | pod and port choice (pure) | `plan` |
//! | k8s objects to the model, pod set from watch events | `pods` |
//! | service and pod reads, the pod watch | `cluster` |
//! | following the pod set, `TargetGone`, switching pods | `monitor` |
//! | accept loop, task ownership | `session` |
//! | one local connection to one pod connection | `bridge` |
//! | status publishing | `hub`, [`ForwardHandle`] |
//! | error mapping | `error` |
//!
//! # How a forward behaves
//!
//! * **One websocket per connection.** Each accepted local connection opens its own
//!   `portforward` and copies bytes both ways with `copy_bidirectional`; a failure on one
//!   connection does not touch the others. The pod's error channel becomes
//!   [`ForwardStatus::Error`](oxikube_domain::ForwardStatus::Error); the next connection
//!   that works restores `Listening`.
//! * **Service targets.** The requested port is a *service* port; its `targetPort` (a number,
//!   or a container port name looked up on the chosen pod) is what is dialed. The oldest ready
//!   pod behind the selector serves, and the forward stays on it until it cannot serve.
//! * **Restart hook.** One pod watch per forward (field selector by name for a pod, label
//!   selector for a service). When the serving pod is deleted, terminating or stops being
//!   usable, `TargetGone` is published. A service forward then moves to another ready pod as
//!   soon as the watch reports one (new connections are closed until then; there is no polling
//!   and no retry loop). A pod forward ends with `Stopped` and leaves the decision to the
//!   manager. A pod target counts as usable while it is running and not terminating:
//!   readiness is not required, so a failing readiness probe does not tear down a forward
//!   someone opened to debug it. A service target needs `Ready`.
//! * **Ownership.** [`ForwardHandle`] owns the task that owns the listener and every bridge;
//!   dropping it aborts them all and frees the port. Dropping a stream from
//!   [`forward`](oxikube_ports::PortForwardPort::forward) aborts its `Portforwarder`.
//!
//! # Policy
//!
//! A forward does not mutate the cluster, so a read-only session may forward; whether it
//! should is the caller's decision (`Capability::PortForward`, E15). The default bind is
//! loopback; binding elsewhere logs a warning.

mod bridge;
mod cluster;
mod connect;
mod error;
mod handle;
mod hub;
mod monitor;
mod plan;
mod pods;
mod session;
#[cfg(test)]
mod tests;

pub use connect::KubePortForward;
pub use handle::ForwardHandle;
