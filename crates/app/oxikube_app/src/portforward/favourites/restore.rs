//! Restoring a cluster's favourites: a bounded burst of starts, each failure told to the
//! notification centre instead of blocking anything.

use futures::StreamExt as _;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::notification::{ForwardPhase, PortForwardChange};

use super::model::FavouriteForward;
use super::service::Inner;
use crate::portforward::{ForwardInfo, ForwardState, StartError};

/// How many favourites start at once, so reconnecting to a cluster with many does not hammer its
/// API server.
pub const RESTORE_CONCURRENCY: usize = 4;

/// What one restore did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RestoreReport {
    /// Favourites now forwarding (or waiting for a pod to become ready).
    pub started: Vec<FavouriteForward>,
    /// Favourites already forwarded, left alone.
    pub already_running: usize,
    /// Favourites whose pod or Service no longer exists (the user is told to edit them).
    pub missing: Vec<FavouriteForward>,
    /// Favourites that did not start, with why (a busy port, no permission, a bad port).
    pub failed: Vec<(FavouriteForward, String)>,
    /// Favourites not started because the cluster is read-only and the setting is off.
    pub read_only: Vec<FavouriteForward>,
}

pub(super) async fn restore(inner: &Inner, cluster: &ClusterId) -> OxiResult<RestoreReport> {
    let mut report = RestoreReport::default();
    let running = inner.manager.list();
    let mut due = Vec::new();
    for favourite in inner.store.list_for(cluster).await? {
        if !favourite.auto_start {
            continue;
        }
        let forwarded = running.iter().any(|f| {
            f.spec.target == favourite.spec.target
                && f.spec.remote_port == favourite.spec.remote_port
        });
        if forwarded {
            report.already_running += 1;
        } else {
            due.push(favourite);
        }
    }
    let manager = &inner.manager;
    let results: Vec<_> = futures::stream::iter(due)
        .map(|favourite| async move {
            let result = manager.start_and_wait(favourite.spec.clone()).await;
            (favourite, result)
        })
        .buffer_unordered(RESTORE_CONCURRENCY)
        .collect()
        .await;
    for (favourite, result) in results {
        match result {
            Ok(info) if target_missing(&info) => {
                manager.remove(info.id);
                report.missing.push(favourite);
            }
            Ok(_) => report.started.push(favourite),
            Err(StartError::ReadOnly(_)) => report.read_only.push(favourite),
            Err(error) => {
                let why = match error.suggestion() {
                    Some(hint) => format!("{error}. {hint}"),
                    None => error.to_string(),
                };
                report.failed.push((favourite, why));
            }
        }
    }
    tell(inner, cluster, &report);
    Ok(report)
}

/// Whether `info` waits for a pod or Service that does not exist (a renamed pod, a deleted
/// Service) rather than for one that is not ready yet.
fn target_missing(info: &ForwardInfo) -> bool {
    matches!(&info.state, ForwardState::Reconnecting { reason, .. } if reason_is_missing(reason))
}

/// The resolver's wording for a target that is not there ("the pod does not exist").
pub(super) fn reason_is_missing(reason: &str) -> bool {
    reason.ends_with("does not exist")
}

/// Tells the notification centre about what did not start: one row per favourite that failed or
/// whose target is gone, one for all the read-only ones of the cluster.
fn tell(inner: &Inner, cluster: &ClusterId, report: &RestoreReport) {
    let Some(notify) = &inner.notify else { return };
    let send =
        |key: String, label: String, favourite: Option<&FavouriteForward>, reason: String| {
            notify.port_forward_changed(PortForwardChange {
                cluster: cluster.clone(),
                forward_id: key,
                label,
                target: favourite.map(|f| f.spec.target.clone()),
                // A favourite that never came up reads as a forward that gave up.
                from: ForwardPhase::Active,
                to: ForwardPhase::Failed,
                reason: Some(reason),
            });
        };
    for favourite in &report.missing {
        send(
            format!("favourite-{}", favourite.id),
            format!("{} ({})", favourite.name, favourite.label()),
            Some(favourite),
            format!(
                "{} {} no longer exists. Edit or remove the favourite.",
                favourite.spec.target.gvk.kind, favourite.spec.target.name
            ),
        );
    }
    for (favourite, why) in &report.failed {
        send(
            format!("favourite-{}", favourite.id),
            format!("{} ({})", favourite.name, favourite.label()),
            Some(favourite),
            format!("The favourite did not start: {why}"),
        );
    }
    if !report.read_only.is_empty() {
        let names: Vec<_> = report.read_only.iter().map(|f| f.name.as_str()).collect();
        send(
            format!("favourites-readonly-{cluster}"),
            format!("{} favourite(s) not started", names.len()),
            None,
            format!(
                "{cluster} is read-only, so {} was not started. Lift read-only mode, or set portforward.allowed_in_readonly.",
                names.join(", ")
            ),
        );
    }
}
