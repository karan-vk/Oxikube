//! Tools for hosted agents (E08-S09): the [`ToolRegistry`] and the first feature tool in it,
//! `k8s.get_logs`.
//!
//! Plain async Rust over the ports: no gpui, no kube, no `rmcp` or `agent-client-protocol`
//! (ADR 0008). `oxikube_mcp` lists and calls these tools; the agent panel offers the same ones to
//! ACP agents.
//!
//! | Piece | Where |
//! |---|---|
//! | tools by name, listed and invoked after a schema check | [`ToolRegistry`], [`RegisterToolError`] (`registry`) |
//! | the schema subset arguments are checked against | [`validate_args`] (`schema`) |
//! | `k8s.get_logs`: a pod's or a selector's newest log lines, bounded and redacted | [`k8s::get_logs`] |
//! | `k8s.top_pods`, `k8s.top_nodes`, `k8s.query_promql` (E13-S12): live usage and Prometheus history, bounded, with absence as a visible result | [`metrics`] |

pub mod k8s;
pub mod metrics;
mod registry;
mod schema;

pub use registry::{RegisterToolError, ToolRegistry};
pub use schema::validate_args;
