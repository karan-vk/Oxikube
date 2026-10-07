//! `@logs` (E08-S09): the log context provider, and "Send to agent" for a viewer's selection.
//!
//! | Piece | Where |
//! |---|---|
//! | the `@logs/...` grammar | [`LogMention`] (`mention`) |
//! | a mention read into a block | [`LogContextProvider`] |
//! | a viewer's selected lines as a block | [`selection_context`] (`selection`) |
//! | the block's header and notes | `block` |

mod block;
mod mention;
mod selection;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_domain::agent::ContextBlock;
use oxikube_ports::{ContextProviderPort, ContextScope, Mention};

use self::block::{block, header, lines_budget};
pub use self::mention::LogMention;
pub use self::selection::selection_context;
use super::pending::ContextSource;
use crate::logs::{
    DEFAULT_TAIL, ExcerptRequest, ExcerptSource, LogClusters, LogFilter, LogService,
};

/// The mention prefix this provider owns: `@logs/...`.
pub const LOGS_PREFIX: &str = "logs";

/// Resolves `@logs/<namespace>/<pod>[/--since/10m]` (a pod, or a workload's pods merged) to one
/// [`ContextBlock`] with the newest lines, a header that says where they came from, and a note for
/// whatever was left out: lines beyond the tail, the size budget, a search limited to the newest
/// lines, failed streams. Secrets are masked best-effort ([`LogService::read_excerpt`]).
///
/// The read does not follow, is bounded by [`ContextScope::max_total_bytes`] (and 64 KiB), and
/// runs on the log service's runtime: await [`resolve`](ContextProviderPort::resolve) off the UI
/// thread. Reading logs is allowed on a read-only cluster.
pub struct LogContextProvider {
    clusters: Arc<dyn LogClusters>,
    logs: Arc<LogService>,
}

impl LogContextProvider {
    /// A provider reading through `logs`, on the cluster `clusters` names.
    pub fn new(clusters: Arc<dyn LogClusters>, logs: Arc<LogService>) -> Self {
        Self { clusters, logs }
    }
}

impl std::fmt::Debug for LogContextProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogContextProvider").finish_non_exhaustive()
    }
}

#[async_trait]
impl ContextProviderPort for LogContextProvider {
    fn mention_prefix(&self) -> &str {
        LOGS_PREFIX
    }

    async fn resolve(
        &self,
        mention: &Mention,
        scope: &ContextScope,
    ) -> OxiResult<Vec<ContextBlock>> {
        let parsed = LogMention::parse(mention.path(), scope.default_namespace.as_deref())?;
        let cluster = self.clusters.cluster(scope.cluster.as_ref())?;

        let mut request = ExcerptRequest::new(parsed.source.clone())
            .tail(parsed.tail.unwrap_or(DEFAULT_TAIL))
            .max_bytes(lines_budget(scope.max_total_bytes));
        if let Some(since) = parsed.since {
            request = request.since(since);
        }
        if let Some(grep) = &parsed.grep {
            request = request.matching(LogFilter::new(grep.clone()));
        }
        let excerpt = self.logs.read_excerpt(cluster.ports, &request).await?;

        let (namespace, subject, container) = match &parsed.source {
            ExcerptSource::Pod {
                namespace,
                pod,
                container,
            } => (namespace.clone(), pod.clone(), container.clone()),
            ExcerptSource::Workload(spec) => {
                (spec.namespace.clone(), spec.label(), spec.container.clone())
            }
        };
        let source = ContextSource {
            cluster: cluster.id,
            cluster_name: cluster.title,
            namespace,
            subject,
            container,
            span: excerpt.span,
            lines: excerpt.lines,
        };
        let since = parsed
            .since
            .map(|since| format!(", since {}", short(since)))
            .unwrap_or_default();
        let title = format!(
            "Logs {}/{} ({} lines{since})",
            source.namespace, source.subject, excerpt.lines
        );
        let cut = excerpt.omitted > 0 || excerpt.budget_cut || excerpt.timed_out;
        Ok(vec![block(
            title,
            &header(&source),
            &excerpt.notes(),
            &excerpt.text,
            cut,
        )])
    }
}

/// A duration in the largest unit that holds it exactly: `90s`, `10m`, `2h`, `1d`.
fn short(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    for (unit, name) in [(86_400, 'd'), (3_600, 'h'), (60, 'm')] {
        if secs.is_multiple_of(unit) {
            return format!("{}{name}", secs / unit);
        }
    }
    format!("{secs}s")
}
