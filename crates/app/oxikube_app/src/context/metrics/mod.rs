//! `@metrics` (E13-S12): a pod's recent CPU and memory as a block of text for an agent prompt.
//!
//! `@metrics/pod/<namespace>/<name>` resolves to one [`ContextBlock`]: the newest samples of the
//! pod from the live metrics ring buffer (up to [`MAX_SAMPLES`](render::MAX_SAMPLES) of the 60
//! it keeps), its requests and limits with the latest usage as a percent of each, and its
//! containers. A cluster nobody is watching has no ring, so the provider takes one reading on
//! demand and says that it has no history. A pod that does not exist, or a cluster whose metrics
//! are unavailable, is a block whose body says so (`# error: ...`, `metrics unavailable:
//! <reason>`), so the prompt carries the reason instead of a silent gap; a mention of the wrong
//! shape is a validation error.
//!
//! Samples older than [`STALE_AFTER`](render::STALE_AFTER) (polling stops when nothing observes
//! the cluster, and the ring is kept) are quoted as `latest`, not `now`, with their age.
//!
//! Read-only, allowed on a read-only cluster. Nothing here is a secret; the reason text still
//! passes through `oxikube_domain::redact`. The body is cut to the caller's byte budget oldest
//! sample first, and the block says how many were left out.

mod mention;
mod render;
#[cfg(test)]
mod tests;

use async_trait::async_trait;
use oxikube_domain::agent::ContextBlock;
use oxikube_domain::ids::Gvk;
use oxikube_domain::metrics::MetricsSubject;
use oxikube_domain::redact::redact;
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::utilisation::{GpuVendors, PodResources, aggregate_pod_requests_limits};
use oxikube_domain::{ErrorKind, OxiResult};
use oxikube_ports::{ContextProviderPort, ContextScope, Mention, MetricsOutcome};

pub use self::mention::MetricsMention;
use self::render::{Origin, PodMetrics, block, error_block};
use crate::metrics::live::{MetricsKind, UsagePoint, reason_text};
use crate::tools::metrics::{MetricsAgentDeps, session_for, within_snapshot_deadline};

/// The mention prefix this provider owns: `@metrics/...`.
pub const METRICS_PREFIX: &str = "metrics";

/// Resolves `@metrics/pod/<namespace>/<name>`: see the [module docs](self).
pub struct MetricsContextProvider {
    deps: MetricsAgentDeps,
}

impl MetricsContextProvider {
    /// A provider reading through `deps`.
    pub fn new(deps: MetricsAgentDeps) -> Self {
        Self { deps }
    }
}

impl std::fmt::Debug for MetricsContextProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricsContextProvider")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ContextProviderPort for MetricsContextProvider {
    fn mention_prefix(&self) -> &str {
        METRICS_PREFIX
    }

    async fn resolve(
        &self,
        mention: &Mention,
        scope: &ContextScope,
    ) -> OxiResult<Vec<ContextBlock>> {
        let target = MetricsMention::parse(mention.path(), scope.default_namespace.as_deref())?;
        let session = session_for(&self.deps.sessions, scope.cluster.as_ref())?;
        let (cluster, title) = (session.id().clone(), session.title().to_owned());
        let (namespace, pod) = (target.namespace.as_str(), target.pod.as_str());
        let not_found = || {
            vec![error_block(
                namespace,
                pod,
                &format!("pod {namespace}/{pod} was not found in {title}"),
            )]
        };

        // The pod itself: it must exist, and its spec gives the requests and limits.
        let resources: Result<PodResources, String> = match session.resources() {
            None => Err("the cluster is not connected".to_owned()),
            Some(reader) => {
                let kind = Gvk::new("", "v1", "Pod");
                let get = reader.get_opt(&kind, Some(namespace), pod);
                match within_snapshot_deadline(self.deps.clock.as_ref(), get).await {
                    None => Err("the pod could not be read in time".to_owned()),
                    Some(Ok(Some(found))) => {
                        aggregate_pod_requests_limits(&found, &GpuVendors::none())
                            .map_err(|e| e.to_string())
                    }
                    Some(Ok(None)) => return Ok(not_found()),
                    Some(Err(error)) if error.kind() == ErrorKind::NotFound => {
                        return Ok(not_found());
                    }
                    Some(Err(error)) => Err(redact(error.message()).into_owned()),
                }
            }
        };

        // History from the ring buffer; with none (nobody is watching), one reading now.
        let usage = self.deps.live.usage(&cluster);
        let mut points = usage.pod_history(namespace, pod);
        let mut containers = usage.containers(namespace, pod);
        let mut origin = Origin::History;
        if points.is_empty() {
            origin = Origin::OneReading;
            let selection = NamespaceSelection::from_names([namespace]);
            let read = self.deps.live.pod_snapshot(&cluster, &selection);
            let outcome = match within_snapshot_deadline(self.deps.clock.as_ref(), read).await {
                None => {
                    return Ok(vec![error_block(
                        namespace,
                        pod,
                        "metrics unavailable: the metrics API did not answer in time",
                    )]);
                }
                Some(Err(error)) => {
                    return Ok(vec![error_block(
                        namespace,
                        pod,
                        &format!("metrics unavailable: {}", redact(error.message())),
                    )]);
                }
                Some(Ok(outcome)) => outcome,
            };
            match outcome {
                MetricsOutcome::Unavailable(why) => {
                    return Ok(vec![error_block(
                        namespace,
                        pod,
                        &format!("metrics unavailable: {}", reason_text(why)),
                    )]);
                }
                MetricsOutcome::Available(samples) => {
                    let found = samples.iter().find(|s| match &s.subject {
                        MetricsSubject::Pod { pod: p } => {
                            p.namespace.as_deref() == Some(namespace) && &*p.name == pod
                        }
                        _ => false,
                    });
                    let Some(sample) = found else {
                        return Ok(vec![error_block(
                            namespace,
                            pod,
                            &format!(
                                "metrics unavailable: metrics-server has no sample for \
                                 {namespace}/{pod} yet (it is new, or not running)"
                            ),
                        )]);
                    };
                    points = vec![UsagePoint::new(sample.ts, sample.cpu, sample.memory)];
                    containers.clone_from(&sample.containers);
                }
            }
        }
        // A watched cluster whose last poll failed still has its older ring: say so.
        let poll_note = (origin == Origin::History)
            .then(|| usage.state(MetricsKind::Pods).unavailable_text())
            .flatten();

        // Polling stops when nothing observes the cluster, but the ring is kept: say how old the
        // newest sample is rather than let it pass as current.
        let now = self.deps.clock.now();
        let stale_for = (origin == Origin::History)
            .then(|| points.last())
            .flatten()
            .map(|last| now.duration_since(last.at))
            .filter(|age| *age > render::STALE_AFTER);

        let metrics = PodMetrics {
            cluster: &title,
            cluster_id: cluster.as_ref(),
            namespace,
            pod,
            points: &points,
            origin,
            poll_note: poll_note.as_deref(),
            stale_for,
            containers: &containers,
            resources: resources.as_ref().map_err(String::as_str),
        };
        Ok(vec![block(&metrics, scope.max_total_bytes)])
    }
}
