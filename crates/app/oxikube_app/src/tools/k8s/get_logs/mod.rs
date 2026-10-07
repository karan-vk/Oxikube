//! `k8s.get_logs` (E08-S09): a read-only tool that returns the newest lines of a pod's log, or of
//! the pods a selector picks merged by server timestamp, as bounded, redacted text.
//!
//! | Argument | Meaning |
//! |---|---|
//! | `pod` or `selector` (exactly one) | one pod; or a label selector (`app=web`) / workload (`deployment/api`) |
//! | `namespace` | default `default` |
//! | `container` | the pod's default container when absent |
//! | `since` | `90s`, `10m`, `2h`, `1h30m`, `1d` (at most 7 days) |
//! | `tail` | the most lines returned, 1 to 2 000, default 200 |
//! | `grep` | a regular expression, case-insensitive: only matching lines (searches the newest 10 000 lines of each container) |
//!
//! The read never follows: it opens a [`LogService`](crate::logs::LogService) session that ends,
//! waits at most 20 s, and returns at most 256 KiB, each line prefixed with its server time and
//! `pod/container`. What was left out is said in a `note:` line. Secrets are masked best-effort
//! before anything leaves the tool; the viewer shows the user's own lines unmasked. The tool is
//! read-only (no risk, no `MutationGuard`) and needs the logs capability. Arguments that break the
//! contract are a failed call (`Err`); a pod that does not exist or a denied `pods/log` is a tool
//! error the model sees (`is_error`).

mod args;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::redact::redact;
use oxikube_domain::{Capabilities, ErrorKind, OxiResult};
use oxikube_ports::{
    ContentPart, ToolAnnotations, ToolContext, ToolDef, ToolName, ToolOutput, ToolPort,
};
use serde_json::{Value, json};

use self::args::GetLogsArgs;
use crate::logs::{DEFAULT_TAIL, LogClusters, LogExcerpt, LogService, MAX_EXCERPT_BYTES, MAX_TAIL};
use crate::tools::{RegisterToolError, ToolRegistry};

/// The tool's name.
pub const GET_LOGS: &str = "k8s.get_logs";

/// The `k8s.get_logs` tool: see the [module docs](self).
pub struct GetLogsTool {
    def: ToolDef,
    clusters: Arc<dyn LogClusters>,
    logs: Arc<LogService>,
}

impl GetLogsTool {
    /// The tool, reading through `logs` on the cluster `clusters` names (the call's cluster, else
    /// the only connected one).
    pub fn new(clusters: Arc<dyn LogClusters>, logs: Arc<LogService>) -> Self {
        Self {
            def: definition(),
            clusters,
            logs,
        }
    }

    /// Registers the tool on `registry`.
    ///
    /// # Errors
    ///
    /// [`RegisterToolError`] when `k8s.get_logs` is registered already.
    pub fn register(
        registry: &ToolRegistry,
        clusters: Arc<dyn LogClusters>,
        logs: Arc<LogService>,
    ) -> Result<(), RegisterToolError> {
        registry.register(Arc::new(Self::new(clusters, logs)))
    }
}

impl std::fmt::Debug for GetLogsTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GetLogsTool").finish_non_exhaustive()
    }
}

#[async_trait]
impl ToolPort for GetLogsTool {
    fn def(&self) -> &ToolDef {
        &self.def
    }

    async fn invoke(&self, args: Value, ctx: &ToolContext) -> OxiResult<ToolOutput> {
        let request = GetLogsArgs::parse(args)?.into_request()?;
        let cluster = match self.clusters.cluster(ctx.cluster.as_ref()) {
            Ok(cluster) => cluster,
            Err(error) if error.kind() == ErrorKind::Validation => return Err(error),
            Err(error) => return Ok(failure(&error)),
        };
        match self.logs.read_excerpt(cluster.ports, &request).await {
            Ok(excerpt) => Ok(output(&excerpt)),
            // A bad pattern is the caller's mistake; the rest ran and failed.
            Err(error) if error.kind() == ErrorKind::Validation => Err(error),
            Err(error) => Ok(failure(&error)),
        }
    }
}

/// A tool error the model sees: what failed, redacted.
fn failure(error: &oxikube_domain::OxiError) -> ToolOutput {
    ToolOutput::error(format!("{}: {}", error.kind(), redact(error.message())))
}

/// The text the model reads (notes first, then the lines) and the same facts as JSON.
fn output(excerpt: &LogExcerpt) -> ToolOutput {
    let notes = excerpt.notes();
    let mut text = String::with_capacity(excerpt.text.len() + 256);
    if excerpt.is_empty() {
        text.push_str("No log lines matched.\n");
    }
    for note in &notes {
        text.push_str("note: ");
        text.push_str(note);
        text.push('\n');
    }
    text.push_str(&excerpt.text);
    ToolOutput {
        content: vec![ContentPart::text(text)],
        structured: Some(json!({
            "lines": excerpt.lines,
            "matched": excerpt.matched,
            "scanned": excerpt.scanned,
            "omitted": excerpt.omitted,
            "truncated": excerpt.omitted > 0 || excerpt.budget_cut || excerpt.timed_out,
            "streams": excerpt.streams,
            "notes": notes,
        })),
        is_error: false,
    }
}

/// The tool's definition: name, schemas, and its read-only, logs-capability contract.
fn definition() -> ToolDef {
    let name = ToolName::new(GET_LOGS).expect("a well-formed tool name");
    ToolDef::read_only(
        name,
        format!(
            "Read the newest log lines of a pod, or of every pod a label selector or workload \
             picks (merged by timestamp). Give exactly one of `pod` and `selector`. Returns at \
             most `tail` lines (default {DEFAULT_TAIL}, max {MAX_TAIL}) and {} KiB, each as \
             `<server time> <pod>/<container> <text>`, with notes about anything left out. It \
             never follows. Secrets are masked best-effort; use `since` and `grep` to narrow.",
            MAX_EXCERPT_BYTES / 1024
        ),
        json!({
            "type": "object",
            "properties": {
                "pod": {
                    "type": "string",
                    "description": "Pod name. Exclusive with `selector`."
                },
                "selector": {
                    "type": "string",
                    "description": "A label selector (`app=web,tier=api`) or a workload \
                        (`deployment/api`, `statefulset/db`, `job/migrate`, `service/web`) whose \
                        pods are read. Exclusive with `pod`."
                },
                "namespace": {
                    "type": "string",
                    "description": "Namespace of the pod or workload. Default `default`."
                },
                "container": {
                    "type": "string",
                    "description": "Container name. Default: the pod's default container (every \
                        container for a selector)."
                },
                "since": {
                    "type": "string",
                    "description": "Only lines newer than this: `90s`, `10m`, `2h`, `1h30m`, \
                        `1d` (at most 7 days)."
                },
                "tail": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_TAIL,
                    "description": format!("Most lines returned, newest last. Default {DEFAULT_TAIL}.")
                },
                "grep": {
                    "type": "string",
                    "description": "Regular expression (case-insensitive): only matching lines. \
                        The newest 10000 lines of each container are searched."
                }
            },
            "additionalProperties": false
        }),
    )
    .with_title("Get Logs")
    .with_needs(Capabilities::LOGS)
    .with_annotations(ToolAnnotations {
        idempotent: true,
        open_world: false,
    })
    .with_output_schema(json!({
        "type": "object",
        "properties": {
            "lines": {"type": "integer", "description": "Lines in the text."},
            "matched": {"type": "integer", "description": "Lines that matched `grep` among those read."},
            "scanned": {"type": "integer", "description": "Lines read."},
            "omitted": {"type": "integer", "description": "Older matching lines left out."},
            "truncated": {"type": "boolean", "description": "Whether anything was left out."},
            "streams": {"type": "integer", "description": "Containers read."},
            "notes": {"type": "array", "items": {"type": "string"}}
        }
    }))
}
