//! The drain itself: plan, cordon, evict a few pods at a time, report as a stream.

use std::sync::Arc;

use futures::channel::mpsc;
use futures::{FutureExt as _, Stream, StreamExt as _, future, stream};
use oxikube_domain::ids::Gvk;
use oxikube_domain::{ErrorKind, OxiError, OxiResult, Resource};
use oxikube_ports::{ListOptions, ResourcePort, WriteOptions};
use tokio::time::Instant;
use tracing::debug;

use super::evict::{Events, Shared, evict_and_wait};
use super::options::{DrainOptions, DrainPlan, DrainProgress, DrainSummary, PodRef};
use super::plan::plan_drain;
use crate::subresource::ResourcePatch;

/// Pods are listed this many at a time.
const PAGE: u32 = 500;

/// How many blocking pods an error names.
const NAMED: usize = 5;

fn node_gvk() -> Gvk {
    Gvk::new("", "v1", "Node")
}

fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

/// Drains `node`: cordons it and evicts its pods, reporting each step as it happens.
///
/// The stream's items are [`DrainProgress`] in order: `Planned`, `Cordoned`, then per pod
/// `Evicting`, `Blocked` (while a budget refuses), `Evicted`, `Gone` or `PodFailed`, and last
/// `Finished`. A failure of the drain as a whole (node missing or forbidden, a pod that
/// blocks the drain, the cordon refused) is a single `Err` item that ends the stream; a pod
/// that cannot be evicted is `PodFailed` and the drain goes on with the others, so check
/// [`DrainSummary::is_complete`] (or use [`drain_to_completion`]).
///
/// Nothing runs until the stream is polled and dropping it stops the drain at once (an
/// eviction already sent stays sent; the node stays cordoned). Pods are evicted
/// [`concurrency`](DrainOptions::concurrency) at a time, on the caller's task: nothing is
/// spawned and nothing blocks.
///
/// With [`dry_run`](DrainOptions::dry_run) only the reads happen.
///
/// **Mutating: reachable only through `MutationGuard`**, with the node's name typed to confirm.
pub fn drain(
    port: Arc<dyn ResourcePort>,
    node: impl Into<String>,
    options: DrainOptions,
) -> impl Stream<Item = OxiResult<DrainProgress>> + Send + 'static {
    let node = node.into();
    let (events, progress) = mpsc::unbounded();
    let work = async move {
        if let Err(error) = execute(&*port, &node, &options, &events).await {
            let _ = events.unbounded_send(Err(error));
        }
    };
    // `work` yields nothing; selecting it with the channel drives it while the stream is
    // polled, and the channel closes when it finishes (it owns the only sender).
    let driver = work
        .into_stream()
        .filter_map(|()| future::ready(None::<OxiResult<DrainProgress>>));
    stream::select(progress, driver)
}

/// Runs a [`drain`] stream to its end and returns the summary.
///
/// # Errors
///
/// The stream's own error; `Conflict` naming the pods left if any pod failed (call [`drain`]
/// itself to keep the per-pod reasons); `Internal` if the stream ended without finishing.
pub async fn drain_to_completion<S>(progress: S) -> OxiResult<DrainSummary>
where
    S: Stream<Item = OxiResult<DrainProgress>>,
{
    futures::pin_mut!(progress);
    while let Some(step) = progress.next().await {
        if let DrainProgress::Finished(summary) = step? {
            return if summary.is_complete() {
                Ok(summary)
            } else {
                Err(OxiError::new(
                    ErrorKind::Conflict,
                    format!(
                        "drain of node {} left {} pod(s): {}",
                        summary.node,
                        summary.failed.len(),
                        names(summary.failed.iter().map(ToString::to_string)),
                    ),
                ))
            };
        }
    }
    Err(OxiError::internal("the drain ended without a result"))
}

async fn execute(
    port: &dyn ResourcePort,
    node: &str,
    options: &DrainOptions,
    events: &Events,
) -> OxiResult<()> {
    let node_object = port.get(&node_gvk(), None, node).await?;
    let pods = pods_on(port, node).await?;
    let plan = plan_drain(&pods, options);
    if !plan.blocked.is_empty() {
        return Err(blocked_error(node, &plan));
    }
    let announce = |progress| {
        let _ = events.unbounded_send(Ok(progress));
    };
    announce(DrainProgress::Planned {
        evict: plan.evict.clone(),
        skipped: plan.skipped.clone(),
    });
    let summary = |evicted, failed| DrainSummary {
        node: node.to_owned(),
        evicted,
        skipped: plan.skipped.clone(),
        failed,
        dry_run: options.dry_run,
    };
    if options.dry_run {
        announce(DrainProgress::Finished(summary(
            plan.evict.clone(),
            Vec::new(),
        )));
        return Ok(());
    }

    let already = node_object.get_bool("/spec/unschedulable") == Some(true);
    if !already {
        debug!(op = "drain_cordon", node, "algorithm");
        port.patch(
            &node_gvk(),
            None,
            node,
            &ResourcePatch::Cordon.to_patch(),
            &WriteOptions::default(),
        )
        .await?;
    }
    announce(DrainProgress::Cordoned { already });

    let shared = Shared {
        port,
        options,
        deadline: Instant::now() + options.timeout,
        events,
    };
    let shared = &shared;
    let outcomes: Vec<(PodRef, bool)> = stream::iter(plan.evict.clone())
        .map(|pod| async move {
            let gone = evict_and_wait(shared, &pod).await;
            (pod, gone)
        })
        .buffer_unordered(options.concurrency.max(1))
        .collect()
        .await;
    let (gone, failed): (Vec<_>, Vec<_>) = outcomes.into_iter().partition(|(_, ok)| *ok);
    let pods = |outcomes: Vec<(PodRef, bool)>| -> Vec<PodRef> {
        outcomes.into_iter().map(|(pod, _)| pod).collect()
    };
    announce(DrainProgress::Finished(summary(pods(gone), pods(failed))));
    Ok(())
}

/// Every pod on `node`, across namespaces.
async fn pods_on(port: &dyn ResourcePort, node: &str) -> OxiResult<Vec<Resource>> {
    let mut options = ListOptions::default()
        .fields(format!("spec.nodeName={node}"))
        .limit(PAGE);
    let mut pods = Vec::new();
    loop {
        let page = port.list(&pod_gvk(), None, &options).await?;
        let next = page.continue_token.clone().filter(|t| !t.is_empty());
        pods.extend(
            page.items
                .into_iter()
                .filter(|pod| pod.get_str("/spec/nodeName") == Some(node)),
        );
        match next {
            Some(token) => options = options.continue_from(token),
            None => return Ok(pods),
        }
    }
}

fn blocked_error(node: &str, plan: &DrainPlan) -> OxiError {
    let first = &plan.blocked[0];
    let more = plan.blocked.len() - 1;
    let others = if more == 0 {
        String::new()
    } else {
        format!(
            ", and {more} more ({})",
            names(plan.blocked[1..].iter().map(|b| b.pod.to_string()))
        )
    };
    OxiError::validation(format!(
        "cannot drain node {node}: pod {} {}{others}",
        first.pod,
        first.reason.explain(),
    ))
}

/// `a, b, c` for the first few names, `a, b, c, ...` when there are more.
fn names(all: impl Iterator<Item = String>) -> String {
    let mut shown: Vec<String> = all.take(NAMED + 1).collect();
    let more = shown.len() > NAMED;
    shown.truncate(NAMED);
    let mut text = shown.join(", ");
    if more {
        text.push_str(", ...");
    }
    text
}
