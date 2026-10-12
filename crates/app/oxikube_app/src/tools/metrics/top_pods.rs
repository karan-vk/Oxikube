//! `k8s.top_pods`: the pods using the most CPU or memory.

use std::fmt::Write as _;
use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::metrics::MetricsSubject;
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{
    ContentPart, MetricsOutcome, ToolAnnotations, ToolContext, ToolDef, ToolName, ToolOutput,
    ToolPort,
};
use serde_json::{Value, json};

use super::bounds::{DEFAULT_LIMIT, MAX_LIMIT, SNAPSHOT_DEADLINE};
use super::cluster::resolve;
use super::deadline::within;
use super::install::MetricsAgentDeps;
use super::top::{Row, newest, rank, sort_and_limit};
use super::unavailable::unavailable;
use crate::metrics::live::reason_text;

/// The tool's name.
pub const TOP_PODS: &str = "k8s.top_pods";

/// The `k8s.top_pods` tool: see the [module docs](super).
pub struct TopPodsTool {
    def: ToolDef,
    deps: MetricsAgentDeps,
}

impl TopPodsTool {
    /// The tool over `deps`.
    pub fn new(deps: MetricsAgentDeps) -> Self {
        Self {
            def: definition(),
            deps,
        }
    }
}

impl std::fmt::Debug for TopPodsTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TopPodsTool").finish_non_exhaustive()
    }
}

#[async_trait]
impl ToolPort for TopPodsTool {
    fn def(&self) -> &ToolDef {
        &self.def
    }

    async fn invoke(&self, args: Value, ctx: &ToolContext) -> OxiResult<ToolOutput> {
        let (sort, limit) = sort_and_limit(&args)?;
        let namespace = match args.get("namespace") {
            None | Some(Value::Null) => None,
            Some(Value::String(name)) if !name.trim().is_empty() => Some(name.trim().to_owned()),
            Some(_) => {
                return Err(OxiError::validation(
                    "`namespace` must be a non-empty string",
                ));
            }
        };
        let session = match resolve(&self.deps.sessions, ctx)? {
            Ok(session) => session,
            Err(output) => return Ok(output),
        };
        let (cluster, title) = (session.id().clone(), session.title().to_owned());
        let selection = namespace.as_deref().map_or(NamespaceSelection::All, |ns| {
            NamespaceSelection::from_names([ns])
        });
        let answer = within(
            self.deps.clock.as_ref(),
            SNAPSHOT_DEADLINE,
            self.deps.live.pod_snapshot(&cluster, &selection),
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

        let mut rows: Vec<Row> = samples
            .iter()
            .filter_map(|sample| {
                let MetricsSubject::Pod { pod } = &sample.subject else {
                    return None;
                };
                Some(Row::of(sample, pod.namespace.as_deref(), &pod.name))
            })
            .collect();
        let total = rows.len();
        rank(&mut rows, sort);
        rows.truncate(limit);
        Ok(render(
            &title,
            namespace.as_deref(),
            sort.id(),
            total,
            &rows,
        ))
    }
}

fn render(
    cluster: &str,
    namespace: Option<&str>,
    sort: &str,
    total: usize,
    rows: &[Row],
) -> ToolOutput {
    let scope = namespace.map_or_else(
        || "all namespaces".to_owned(),
        |ns| format!("namespace {ns}"),
    );
    let sampled_at = newest(rows).map(|at| at.to_string());
    let mut text = format!(
        "top {} of {total} pods by {sort} in {scope} on {cluster}",
        rows.len()
    );
    if let Some(at) = &sampled_at {
        let _ = write!(text, " (sampled {at})");
    }
    text.push('\n');
    if rows.is_empty() {
        text.push_str("no pods have metrics\n");
    } else {
        text.push_str("NAMESPACE\tPOD\tCPU\tMEMORY\n");
    }
    for row in rows {
        let _ = writeln!(
            text,
            "{}\t{}\t{}\t{}",
            row.namespace.as_deref().unwrap_or("-"),
            row.name,
            row.cpu_text(),
            row.memory_text()
        );
    }
    let listed: Vec<Value> = rows
        .iter()
        .map(|row| {
            let mut entry = row.figures();
            entry["namespace"] = json!(row.namespace);
            entry["name"] = json!(row.name);
            entry
        })
        .collect();
    ToolOutput {
        content: vec![ContentPart::text(text)],
        structured: Some(json!({
            "status": "ok",
            "cluster": cluster,
            "namespace": namespace,
            "sort": sort,
            "total_pods": total,
            "returned": rows.len(),
            "sampled_at": sampled_at,
            "pods": listed,
        })),
        is_error: false,
    }
}

fn definition() -> ToolDef {
    let name = ToolName::new(TOP_PODS).expect("a well-formed tool name");
    ToolDef::read_only(
        name,
        "List the pods using the most CPU or memory right now, from metrics-server (like `kubectl \
         top pods`). Values are CPU in millicores and memory in bytes with their sample time. \
         If the cluster has no metrics-server or it cannot be read, the result has \
         status \"unavailable\" and the reason; that is not the same as zero usage.",
        json!({
            "type": "object",
            "properties": {
                "namespace": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Only this namespace. Default: every namespace."
                },
                "sort": {
                    "type": "string",
                    "enum": ["cpu", "memory"],
                    "description": "What to rank by. Default cpu."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_LIMIT,
                    "description": format!("How many pods to return. Default {DEFAULT_LIMIT}.")
                }
            },
            "additionalProperties": false
        }),
    )
    .with_title("Top pods")
    .with_annotations(ToolAnnotations {
        idempotent: true,
        ..ToolAnnotations::default()
    })
}

/// The tool, shared.
pub(super) fn tool(deps: &MetricsAgentDeps) -> Arc<TopPodsTool> {
    Arc::new(TopPodsTool::new(deps.clone()))
}
