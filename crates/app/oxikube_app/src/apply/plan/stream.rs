//! [`PlanStream`]: one plan dispatch, as a stream of per-object results.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures::channel::mpsc::UnboundedReceiver;
use futures::future::BoxFuture;
use futures::{FutureExt as _, Stream, StreamExt as _};
use serde_json::Value;

use super::types::{ObjectResult, PlanReport};
use crate::apply::types::ApplyError;

/// The results of a [`dry_run`](crate::apply::ApplyService::dry_run_plan) or an
/// [`apply`](crate::apply::ApplyService::apply_plan), one per object as it is decided (in completion
/// order: objects of one rank finish in any order). A refusal of the whole plan (read-only,
/// declined, an invalid plan) is one `Err` item, the last; nothing was sent then.
///
/// Polling the stream drives the work, so a view runs it on the Kubernetes runtime
/// (`oxikube_runtime::spawn_kube`) and never on the UI thread. Dropping it stops the work at
/// once (the request in flight is audited as cancelled); the plan's
/// [`cancel`](super::ApplyOptions::cancel) token stops it between objects instead, reporting the
/// rest as skipped.
#[must_use = "a plan stream does nothing unless polled"]
pub struct PlanStream {
    /// The dispatch, until it returns.
    run: Option<BoxFuture<'static, Result<(), ApplyError>>>,
    /// The dispatch's refusal, handed out once the reports are drained.
    refusal: Option<ApplyError>,
    /// The handler's reports. Ends when the dispatch is over (its sender is dropped with it).
    reports: UnboundedReceiver<Value>,
}

impl std::fmt::Debug for PlanStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlanStream")
            .field("running", &self.run.is_some())
            .finish_non_exhaustive()
    }
}

impl PlanStream {
    pub(super) fn new(
        run: BoxFuture<'static, Result<(), ApplyError>>,
        reports: UnboundedReceiver<Value>,
    ) -> Self {
        Self {
            run: Some(run),
            refusal: None,
            reports,
        }
    }

    /// Runs the plan to the end and gathers every result, in plan order.
    ///
    /// # Errors
    ///
    /// The refusal of the whole plan, when there is one.
    pub async fn into_report(mut self) -> Result<PlanReport, ApplyError> {
        let mut report = PlanReport::default();
        while let Some(item) = self.next().await {
            report.objects.push(item?);
        }
        report.sort();
        Ok(report)
    }
}

impl Stream for PlanStream {
    type Item = Result<ObjectResult, ApplyError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;
        if let Some(run) = this.run.as_mut()
            && let Poll::Ready(outcome) = run.poll_unpin(cx)
        {
            this.run = None;
            this.refusal = outcome.err();
        }
        loop {
            return match this.reports.poll_next_unpin(cx) {
                Poll::Ready(Some(report)) => match serde_json::from_value(report) {
                    Ok(result) => Poll::Ready(Some(Ok(result))),
                    Err(error) => {
                        tracing::debug!(%error, "an apply plan report did not read");
                        continue;
                    }
                },
                // The reports end when the dispatch drops its sender; wait for its outcome.
                Poll::Ready(None) if this.run.is_some() => Poll::Pending,
                // The dispatch is over and its reports are drained.
                Poll::Ready(None) => Poll::Ready(this.refusal.take().map(Err)),
                Poll::Pending => Poll::Pending,
            };
        }
    }
}
