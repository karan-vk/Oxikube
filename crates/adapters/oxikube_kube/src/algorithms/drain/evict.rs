//! One pod of a drain: evict it, back off while a budget refuses, wait until it is gone.

use futures::channel::mpsc::UnboundedSender;
use oxikube_domain::ids::Gvk;
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::{DeleteOptions, Preconditions, ResourcePort};
use tokio::time::{Instant, sleep};
use tracing::debug;

use super::options::{DrainOptions, DrainProgress, PodRef};
use crate::subresource::eviction_blocked;

/// Where progress goes. A closed receiver (the stream was dropped) is ignored: the drain is
/// being cancelled and its futures are dropped next.
pub(super) type Events = UnboundedSender<OxiResult<DrainProgress>>;

/// What every pod of one drain shares.
pub(super) struct Shared<'a> {
    pub(super) port: &'a dyn ResourcePort,
    pub(super) options: &'a DrainOptions,
    pub(super) deadline: Instant,
    pub(super) events: &'a Events,
}

impl Shared<'_> {
    pub(super) fn emit(&self, progress: DrainProgress) {
        let _ = self.events.unbounded_send(Ok(progress));
    }
}

fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

/// Evicts `pod` and waits for it to go. Reports every step; `true` if the pod is gone, `false`
/// after a [`DrainProgress::PodFailed`].
pub(super) async fn evict_and_wait(shared: &Shared<'_>, pod: &PodRef) -> bool {
    match run(shared, pod).await {
        Ok(()) => true,
        Err(error) => {
            shared.emit(DrainProgress::PodFailed {
                pod: pod.clone(),
                error,
            });
            false
        }
    }
}

async fn run(shared: &Shared<'_>, pod: &PodRef) -> Result<(), String> {
    let options = shared.options;
    let mut delay = options.retry_initial;
    let mut attempt = 0_u32;
    loop {
        attempt += 1;
        shared.emit(DrainProgress::Evicting {
            pod: pod.clone(),
            attempt,
        });
        let result = shared
            .port
            .evict(&pod.namespace, &pod.name, &delete_options(pod, options))
            .await;
        match result {
            Ok(()) => {
                debug!(op = "drain_evict", %pod, attempt, "algorithm");
                shared.emit(DrainProgress::Evicted { pod: pod.clone() });
                return wait_until_gone(shared, pod).await;
            }
            // Deleted by someone else in the meantime.
            Err(error) if error.kind() == ErrorKind::NotFound => {
                shared.emit(DrainProgress::Gone { pod: pod.clone() });
                return Ok(());
            }
            // The uid precondition failed: the pod was replaced under the same name.
            Err(error) if error.kind() == ErrorKind::Conflict => {
                return if is_gone(shared, pod).await.map_err(|e| e.to_string())? {
                    shared.emit(DrainProgress::Gone { pod: pod.clone() });
                    Ok(())
                } else {
                    Err(error.to_string())
                };
            }
            Err(error) if is_blocked(&error) => {
                let reason = reason_of(&error);
                if Instant::now() + delay > shared.deadline {
                    return Err(format!("still blocked at the timeout: {reason}"));
                }
                shared.emit(DrainProgress::Blocked {
                    pod: pod.clone(),
                    attempt,
                    reason,
                    retry_in: delay,
                });
                sleep(delay).await;
                delay = (delay * 2).min(options.retry_max);
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

/// Polls until the pod is missing (or replaced), or the deadline passes.
async fn wait_until_gone(shared: &Shared<'_>, pod: &PodRef) -> Result<(), String> {
    let poll = shared.options.poll_interval;
    loop {
        if is_gone(shared, pod).await.map_err(|e| e.to_string())? {
            shared.emit(DrainProgress::Gone { pod: pod.clone() });
            return Ok(());
        }
        if Instant::now() + poll > shared.deadline {
            return Err("still terminating at the timeout".to_owned());
        }
        sleep(poll).await;
    }
}

/// Whether the pod is missing or is now another object (a different uid).
async fn is_gone(shared: &Shared<'_>, pod: &PodRef) -> OxiResult<bool> {
    let live = shared
        .port
        .get_opt(&pod_gvk(), Some(&pod.namespace), &pod.name)
        .await?;
    Ok(match (live, &pod.uid) {
        (None, _) => true,
        (Some(live), Some(uid)) => live.meta.uid.as_deref().is_some_and(|live| live != uid),
        (Some(_), None) => false,
    })
}

fn delete_options(pod: &PodRef, options: &DrainOptions) -> DeleteOptions {
    DeleteOptions {
        grace_period_secs: options.grace_period_secs,
        preconditions: pod.uid.clone().map(|uid| Preconditions {
            uid: Some(uid),
            resource_version: None,
        }),
        ..DeleteOptions::default()
    }
}

/// A refusal worth waiting out: a budget's 429, or any other transient failure the adapter marks
/// retryable (rate limiting, a server hiccup).
fn is_blocked(error: &OxiError) -> bool {
    eviction_blocked(error).is_some() || error.is_retryable()
}

fn reason_of(error: &OxiError) -> String {
    match eviction_blocked(error) {
        Some(blocked) => blocked.reason.clone(),
        None => error.to_string(),
    }
}
