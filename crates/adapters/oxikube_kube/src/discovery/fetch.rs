//! Network side of discovery: aggregated first, legacy per-group as the fallback.
//!
//! Aggregated discovery is two requests (`/api`, `/apis`) on Kubernetes 1.26+ (stable in 1.30).
//! The legacy shape is `N + 2` requests; they run concurrently. The requests are the same ones
//! kube's `Discovery::run_aggregated()` / `run()` issue (same `Client` methods); see the module
//! docs of [`super::convert`] for why the documents are read directly.

use futures::{StreamExt, stream};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::APIResourceList;
use kube::Client;
use oxikube_domain::OxiResult;
use tracing::{debug, warn};

use super::convert::{self, Discovered, LegacyList};
use super::error::{from_kube, legacy_may_help};

/// Concurrent requests in the legacy (`N + 2`) shape.
const LEGACY_CONCURRENCY: usize = 8;

/// The result of one discovery run.
pub(super) struct Fetched {
    pub(super) kinds: Vec<Discovered>,
    /// Whether the aggregated endpoints answered (false: the legacy fallback ran).
    pub(super) aggregated: bool,
}

/// Runs discovery against the server; `try_aggregated: false` skips straight to the legacy shape.
pub(super) async fn fetch(client: &Client, try_aggregated: bool) -> OxiResult<Fetched> {
    if !try_aggregated {
        return fetch_legacy(client).await;
    }
    match aggregated(client).await {
        Ok(Some(kinds)) => {
            return Ok(Fetched {
                kinds,
                aggregated: true,
            });
        }
        Ok(None) => debug!("discovery: server ignored aggregated Accept header, using legacy"),
        Err(err) => {
            let err = from_kube(err, "aggregated discovery");
            if !legacy_may_help(&err) {
                return Err(err);
            }
            debug!(error = %err, "discovery: aggregated discovery failed, using legacy");
        }
    }
    fetch_legacy(client).await
}

async fn fetch_legacy(client: &Client) -> OxiResult<Fetched> {
    let kinds = legacy(client)
        .await
        .map_err(|e| from_kube(e, "API discovery"))?;
    Ok(Fetched {
        kinds,
        aggregated: false,
    })
}

/// `Ok(None)` when the server answered with the legacy document shapes instead (pre-1.26
/// servers ignore the aggregated `Accept` header and kube's `items` default hides that).
async fn aggregated(client: &Client) -> Result<Option<Vec<Discovered>>, kube::Error> {
    let (apis, core) = futures::try_join!(
        client.list_api_groups_aggregated(),
        client.list_core_api_versions_aggregated()
    )?;
    if apis.items.is_empty() || core.items.is_empty() {
        return Ok(None);
    }
    let mut groups = core.items;
    groups.extend(apis.items);
    Ok(Some(convert::from_aggregated(&groups)))
}

/// One `APIResourceList` request of the legacy shape.
struct Job {
    group_version: String,
    core: bool,
    preferred: bool,
}

async fn legacy(client: &Client) -> Result<Vec<Discovered>, kube::Error> {
    let (groups, core) =
        futures::try_join!(client.list_api_groups(), client.list_core_api_versions())?;
    let mut jobs = Vec::new();
    for version in core.versions {
        jobs.push(Job {
            preferred: version == "v1",
            group_version: version,
            core: true,
        });
    }
    for group in &groups.groups {
        let preferred = convert::preferred_version(group);
        for version in &group.versions {
            jobs.push(Job {
                preferred: preferred.as_deref() == Some(version.version.as_str()),
                group_version: version.group_version.clone(),
                core: false,
            });
        }
    }
    let total = jobs.len();
    let results: Vec<_> = stream::iter(jobs)
        .map(|job| async move {
            let list = list_resources(client, &job).await;
            (job, list)
        })
        .buffer_unordered(LEGACY_CONCURRENCY)
        .collect()
        .await;

    // A group version that fails (an unavailable aggregated API such as metrics.k8s.io) is
    // skipped, like kubectl does; only a total failure is an error.
    let mut lists = Vec::with_capacity(total);
    let mut first_error = None;
    for (job, result) in results {
        match result {
            Ok(list) => lists.push(LegacyList {
                preferred: job.preferred,
                list,
            }),
            Err(err) => {
                warn!(group_version = %job.group_version, error = %err, "discovery: group version unavailable");
                first_error.get_or_insert(err);
            }
        }
    }
    match first_error {
        Some(err) if lists.is_empty() => Err(err),
        _ => Ok(convert::from_legacy(&lists)),
    }
}

async fn list_resources(client: &Client, job: &Job) -> Result<APIResourceList, kube::Error> {
    if job.core {
        client.list_core_api_resources(&job.group_version).await
    } else {
        client.list_api_group_resources(&job.group_version).await
    }
}
