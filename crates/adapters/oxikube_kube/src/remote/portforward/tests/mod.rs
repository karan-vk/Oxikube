//! Unit tests for port forwarding: the pure resolution rules, the conversions, and the whole
//! session against a scripted cluster and an in-memory pod connector. The kube-backed paths
//! (the websocket itself, the pod watch) are covered by `tests/portforward.rs` on kind.

mod errors;
mod failover;
mod fakes;
mod plan;
mod pods;
mod session;
