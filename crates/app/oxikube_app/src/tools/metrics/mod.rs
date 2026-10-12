//! The metrics tools of the agent (E13-S12): `k8s.top_pods`, `k8s.top_nodes` and
//! `k8s.query_promql`, and the installer that registers them with the `@metrics` mention.
//!
//! Agents read cluster usage the way a user does: live CPU and memory from metrics-server, and
//! history from Prometheus when the cluster has one. All three tools are read-only (no risk, no
//! confirmation tier), stay available on a read-only cluster, and go through the same
//! [`ToolRegistry`](crate::tools::ToolRegistry) as `k8s.get_logs`. The names carry the `k8s.`
//! namespace [`ToolName`](oxikube_ports::ToolName) requires
//! (`top_pods` and `query_promql` are the acceptance's names).
//!
//! | Tool | Arguments | Answers with |
//! |---|---|---|
//! | `k8s.top_pods` | `namespace` (default: all), `sort` (`cpu` or `memory`, default `cpu`), `limit` (1 to 50, default 10) | the busiest pods: CPU in millicores, memory in bytes and `Mi`, the sample time |
//! | `k8s.top_nodes` | `sort`, `limit` | the busiest nodes, plus the percent of what the node offers (`status.allocatable`) when it can be read |
//! | `k8s.query_promql` | `query` (raw PromQL), **or** `scope` + `metric` + `name` (+ `namespace`, `container`, `labels`) from the query catalogue; `range` (`15m`, `6h`, up to `7d`) for a range query, else an instant one | at most 20 series of at most 60 points each, values rounded, with the unit |
//!
//! # Absence is an answer
//!
//! No metrics-server, a denied `metrics.k8s.io`, no Prometheus found, a Prometheus the settings
//! turn off, one that does not answer: each is a normal result with `status: "unavailable"` and
//! `metrics unavailable: <reason>` as its text, never an `Err` and never a zero (the model can
//! tell "nothing is using CPU" from "nobody can say"). A query Prometheus rejects is a tool
//! error the model sees (it can fix the query); a malformed call (bad sort key, a negative limit,
//! an empty query, both query forms) is an `Err`.
//!
//! # Where the numbers come from
//!
//! The `top_*` tools make one on-demand read through [`MetricsService`](crate::MetricsService)
//! ([`pod_snapshot`](crate::MetricsService::pod_snapshot)), so they work while no table is
//! polling and leave the polling cache alone. `query_promql` goes through the app's
//! [`PromqlPort`](oxikube_ports::PromqlPort) (the endpoint, credentials, timeout and response
//! size limit are the adapter's, E13-S04), asks the [`MetricsProviderService`](crate::MetricsProviderService)
//! which stack the cluster has (and whether the settings allow it), and builds catalogue queries
//! with the [`QueryCataloguePort`](oxikube_ports::QueryCataloguePort) (E13-S06). Each call also
//! has a deadline on the app clock, so a stuck backend cannot hold an agent's turn.
//!
//! # Size and redaction
//!
//! Every answer is bounded (`bounds`): rows, series, points per series and query length. Text
//! that came from outside (the query as echoed, series labels, error messages) passes through
//! `oxikube_domain::redact`; a bearer token is never in an answer, because the tools never see
//! one (the adapter reads it from the keychain).

mod bounds;
mod cluster;
mod deadline;
mod install;
mod promql;
mod top;
mod top_nodes;
mod top_pods;
mod unavailable;

#[cfg(test)]
pub(crate) mod tests;

pub use bounds::{
    DEFAULT_LIMIT, MAX_LIMIT, MAX_POINTS_PER_SERIES, MAX_QUERY_CHARS, MAX_RANGE, MAX_SERIES,
};
pub use install::{InstallError, MetricsAgentDeps, install};
pub use promql::{QUERY_PROMQL, QueryPromqlTool};
pub use top_nodes::{TOP_NODES, TopNodesTool};
pub use top_pods::{TOP_PODS, TopPodsTool};

pub(crate) use cluster::session_for;
pub(crate) use deadline::within_snapshot_deadline;
