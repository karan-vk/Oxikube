//! `MetricsPort` on `metrics.k8s.io` (E04-S11, ADR 0011).
//!
//! [`KubeMetrics`] serves one connected cluster. Node and pod usage come from metrics-server
//! through the `k8s-metrics` types (`NodeMetrics`, `PodMetrics`); we never install or probe for
//! anything else. Usage strings are parsed with the domain [`Quantity`](oxikube_domain::Quantity)
//! (exact, every suffix and exponent) and handed over as [`MetricsSample`]s in nanocores and
//! bytes, with the server's sample timestamp and window so the UI can show staleness.
//! Utilisation against allocatable or requests is then `Quantity` maths on the domain side
//! ([`MetricsSample::cpu_quantity`](oxikube_domain::metrics::MetricsSample::cpu_quantity) and
//! friends).
//!
//! | Piece | Where |
//! |---|---|
//! | `MetricsPort::{node_metrics, pod_metrics}` | this file |
//! | list requests, `k8s-metrics` decode, thin fallback decode | `fetch`, `raw`, `duration` |
//! | usage text to `MetricsSample` | `convert` |
//! | 404 / 503 to "absent", the rest to `OxiError` | `error` |
//!
//! # Cost
//!
//! One request per node list, and one per pod list: all namespaces in a single request for
//! [`NamespaceSelection::All`], one request per selected namespace otherwise (the API has no
//! multi-namespace selector), at most `MAX_CONCURRENT_LISTS` in flight. There is no per-pod
//! request. The API has no pagination, so 2 000 pods are one response of roughly 600 to 800 kB.
//!
//! # Absence
//!
//! A 404 on the group (no metrics-server) answers [`MissingReason::NotInstalled`](oxikube_domain::metrics::MissingReason::NotInstalled); a 503 (the
//! APIService is registered but its backend is not serving) answers
//! [`MissingReason::Unavailable`](oxikube_domain::metrics::MissingReason::Unavailable); both come back as `Ok(MetricsOutcome::Unavailable(..))`.
//! A 403 is `Err(Forbidden)`, and timeouts and connection failures are retryable
//! `Network` / `Timeout` errors, so the UI can tell "forbidden" and "try again" from "not
//! installed". The group is not probed through discovery first: the list itself is the probe and
//! costs no extra request.
//!
//! # Fallback types
//!
//! If a payload does not decode into the crate's types the list is re-read with thin internal
//! decoding (`raw`) that tolerates missing fields; see `fetch`.

mod convert;
mod duration;
mod error;
mod fetch;
mod raw;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use async_trait::async_trait;
use futures::{StreamExt, TryStreamExt, stream};
use jiff::Timestamp;
use kube::Client;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::metrics::MetricsSample;
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{MetricsOutcome, MetricsPort};
use tracing::debug;

use error::Stop;
use raw::RawPod;

/// Most namespaced pod lists in flight at once for a [`NamespaceSelection::Set`].
const MAX_CONCURRENT_LISTS: usize = 4;

/// Node and pod metrics for one cluster. Cheap to clone; clones share the client.
#[derive(Clone)]
pub struct KubeMetrics {
    client: Client,
    cluster: ClusterId,
}

impl KubeMetrics {
    /// Reads `metrics.k8s.io` through `client`, the connection to `cluster`.
    pub fn new(client: Client, cluster: ClusterId) -> Self {
        Self { client, cluster }
    }

    fn check_cluster(&self, requested: &ClusterId) -> OxiResult<()> {
        if requested == &self.cluster {
            return Ok(());
        }
        Err(OxiError::validation(format!(
            "this metrics adapter serves cluster {}, not {requested}",
            self.cluster
        )))
    }

    async fn nodes(&self) -> Result<Vec<MetricsSample>, Stop> {
        let fetched_at = Timestamp::now();
        let nodes = fetch::nodes(&self.client).await?;
        Ok(nodes
            .into_iter()
            .map(|raw| convert::node_sample(raw, fetched_at))
            .collect())
    }

    async fn pods(&self, namespaces: &NamespaceSelection) -> Result<Vec<MetricsSample>, Stop> {
        let fetched_at = Timestamp::now();
        let pods: Vec<RawPod> = match namespaces {
            NamespaceSelection::All => fetch::pods(&self.client, None).await?,
            NamespaceSelection::Set(names) => self.pods_in(names).await?,
        };
        Ok(pods
            .into_iter()
            .map(|raw| convert::pod_sample(&self.cluster, raw, fetched_at))
            .collect())
    }

    /// One list per namespace, a few at a time; the first absence or failure ends the call.
    async fn pods_in(&self, names: &BTreeSet<String>) -> Result<Vec<RawPod>, Stop> {
        let lists: Vec<Vec<RawPod>> = stream::iter(names.iter().cloned())
            .map(|ns| async move { fetch::pods(&self.client, Some(&ns)).await })
            .buffered(MAX_CONCURRENT_LISTS)
            .try_collect()
            .await?;
        Ok(lists.into_iter().flatten().collect())
    }
}

impl std::fmt::Debug for KubeMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KubeMetrics")
            .field("cluster", &self.cluster)
            .finish_non_exhaustive()
    }
}

/// `Ok(Available)`, `Ok(Unavailable)` for absence, `Err` for a failed call.
fn outcome(result: Result<Vec<MetricsSample>, Stop>, what: &str) -> OxiResult<MetricsOutcome> {
    match result {
        Ok(samples) => Ok(MetricsOutcome::Available(samples)),
        Err(Stop::Absent(reason)) => {
            debug!(?reason, "{what}: metrics.k8s.io cannot answer");
            Ok(MetricsOutcome::Unavailable(reason))
        }
        Err(Stop::Failed(err)) => Err(err),
    }
}

#[async_trait]
impl MetricsPort for KubeMetrics {
    async fn node_metrics(&self, cluster: &ClusterId) -> OxiResult<MetricsOutcome> {
        self.check_cluster(cluster)?;
        outcome(self.nodes().await, "node metrics")
    }

    async fn pod_metrics(
        &self,
        cluster: &ClusterId,
        namespaces: &NamespaceSelection,
    ) -> OxiResult<MetricsOutcome> {
        self.check_cluster(cluster)?;
        outcome(self.pods(namespaces).await, "pod metrics")
    }
}
