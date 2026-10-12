//! `k8s.top_nodes`: the nodes using the most CPU or memory.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::ids::Gvk;
use oxikube_domain::metrics::MetricsSubject;
use oxikube_domain::utilisation::{GpuVendors, NodeAllocatable, percent};
use oxikube_domain::{OxiResult, Quantity};
use oxikube_ports::{
    ContentPart, ListOptions, MetricsOutcome, ToolAnnotations, ToolContext, ToolDef, ToolName,
    ToolOutput, ToolPort,
};
use serde_json::{Value, json};

use super::bounds::{DEFAULT_LIMIT, MAX_LIMIT, SNAPSHOT_DEADLINE};
use super::cluster::resolve;
use super::deadline::within;
use super::install::MetricsAgentDeps;
use super::top::{Row, newest, rank, sort_and_limit};
use super::unavailable::unavailable;
use crate::metrics::live::reason_text;
use crate::session::ClusterSession;

/// The tool's name.
pub const TOP_NODES: &str = "k8s.top_nodes";

/// What a node offers, by node name: CPU in nanocores and memory in bytes.
type Allocatable = HashMap<String, NodeAllocatable>;

/// The `k8s.top_nodes` tool: see the [module docs](super).
pub struct TopNodesTool {
    def: ToolDef,
    deps: MetricsAgentDeps,
}

impl TopNodesTool {
    /// The tool over `deps`.
    pub fn new(deps: MetricsAgentDeps) -> Self {
        Self {
            def: definition(),
            deps,
        }
    }

    /// The nodes' allocatable amounts, for the percents. A node list that cannot be read (RBAC,
    /// a slow API) leaves the percents out; the usage is still answered.
    async fn allocatable(&self, session: &ClusterSession) -> Allocatable {
        let Some(reader) = session.resources() else {
            return Allocatable::new();
        };
        let node = Gvk::new("", "v1", "Node");
        let options = ListOptions::default().limit(500);
        let listing = reader.list(&node, None, &options);
        let Some(Ok(page)) = within(self.deps.clock.as_ref(), SNAPSHOT_DEADLINE, listing).await
        else {
            return Allocatable::new();
        };
        let gpus = GpuVendors::none();
        page.items
            .iter()
            .filter_map(|node| NodeAllocatable::from_resource(node, &gpus).ok())
            .map(|alloc| (alloc.name.to_string(), alloc))
            .collect()
    }
}

impl std::fmt::Debug for TopNodesTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TopNodesTool").finish_non_exhaustive()
    }
}

#[async_trait]
impl ToolPort for TopNodesTool {
    fn def(&self) -> &ToolDef {
        &self.def
    }

    async fn invoke(&self, args: Value, ctx: &ToolContext) -> OxiResult<ToolOutput> {
        let (sort, limit) = sort_and_limit(&args)?;
        let session = match resolve(&self.deps.sessions, ctx)? {
            Ok(session) => session,
            Err(output) => return Ok(output),
        };
        let (cluster, title) = (session.id().clone(), session.title().to_owned());
        let answer = within(
            self.deps.clock.as_ref(),
            SNAPSHOT_DEADLINE,
            self.deps.live.node_snapshot(&cluster),
        )
        .await;
        let samples = match answer {
            None => {
                return Ok(unavailable(
                    &title,
                    "the metrics API did not answer in time",
                ));
            }
            Some(Err(error)) => return Ok(unavailable(&title, error.message())),
            Some(Ok(MetricsOutcome::Unavailable(why))) => {
                return Ok(unavailable(&title, reason_text(why)));
            }
            Some(Ok(MetricsOutcome::Available(samples))) => samples,
        };

        let allocatable = self.allocatable(&session).await;
        let mut rows: Vec<Row> = samples
            .iter()
            .filter_map(|sample| {
                let MetricsSubject::Node { node } = &sample.subject else {
                    return None;
                };
                let mut row = Row::of(sample, None, node);
                if let Some(alloc) = allocatable.get(&**node) {
                    row.cpu_percent = percent_of(sample.cpu_quantity(), &alloc.allocatable.cpu);
                    row.memory_percent =
                        percent_of(sample.memory_quantity(), &alloc.allocatable.memory);
                }
                Some(row)
            })
            .collect();
        let total = rows.len();
        rank(&mut rows, sort);
        rows.truncate(limit);
        Ok(render(&title, sort.id(), total, &rows))
    }
}

/// `usage` as a percent of `base`, to one decimal; `None` without a reading or a base.
fn percent_of(usage: Option<Quantity>, base: &Quantity) -> Option<f64> {
    usage
        .and_then(|usage| percent(&usage, base))
        .map(|p| (p * 10.0).round() / 10.0)
}

fn render(cluster: &str, sort: &str, total: usize, rows: &[Row]) -> ToolOutput {
    let sampled_at = newest(rows).map(|at| at.to_string());
    let mut text = format!("top {} of {total} nodes by {sort} on {cluster}", rows.len());
    if let Some(at) = &sampled_at {
        let _ = write!(text, " (sampled {at})");
    }
    text.push('\n');
    if rows.is_empty() {
        text.push_str("no nodes have metrics\n");
    } else {
        text.push_str("NODE\tCPU\tCPU%\tMEMORY\tMEMORY%\n");
    }
    let shown = |p: Option<f64>| p.map_or_else(|| "-".to_owned(), |p| format!("{p:.1}%"));
    let mut listed = Vec::with_capacity(rows.len());
    for row in rows {
        let _ = writeln!(
            text,
            "{}\t{}\t{}\t{}\t{}",
            row.name,
            row.cpu_text(),
            shown(row.cpu_percent),
            row.memory_text(),
            shown(row.memory_percent)
        );
        let mut entry = row.figures();
        entry["name"] = json!(row.name);
        entry["cpu_percent_of_allocatable"] = json!(row.cpu_percent);
        entry["memory_percent_of_allocatable"] = json!(row.memory_percent);
        listed.push(entry);
    }
    ToolOutput {
        content: vec![ContentPart::text(text)],
        structured: Some(json!({
            "status": "ok",
            "cluster": cluster,
            "sort": sort,
            "total_nodes": total,
            "returned": rows.len(),
            "sampled_at": sampled_at,
            "nodes": listed,
        })),
        is_error: false,
    }
}

fn definition() -> ToolDef {
    let name = ToolName::new(TOP_NODES).expect("a well-formed tool name");
    ToolDef::read_only(
        name,
        "List the nodes using the most CPU or memory right now, from metrics-server (like \
         `kubectl top nodes`), with the percent of what each node offers (its allocatable) when \
         that can be read. If the cluster has no metrics-server or it cannot be read, the result \
         has status \"unavailable\" and the reason; that is not the same as zero usage.",
        json!({
            "type": "object",
            "properties": {
                "sort": {
                    "type": "string",
                    "enum": ["cpu", "memory"],
                    "description": "What to rank by. Default cpu."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_LIMIT,
                    "description": format!("How many nodes to return. Default {DEFAULT_LIMIT}.")
                }
            },
            "additionalProperties": false
        }),
    )
    .with_title("Top nodes")
    .with_annotations(ToolAnnotations {
        idempotent: true,
        ..ToolAnnotations::default()
    })
}

/// The tool, shared.
pub(super) fn tool(deps: &MetricsAgentDeps) -> Arc<TopNodesTool> {
    Arc::new(TopNodesTool::new(deps.clone()))
}
