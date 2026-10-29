//! The real pass: the whole plan rehearsed first, then applied rank by rank, a few objects of a
//! rank at a time, each new CustomResourceDefinition `Established` before the custom resources
//! that need it.
//!
//! A failure skips only what needs the failed object (as `kubectl apply`), unless stop on error
//! is set: then a failed dry run applies nothing, and a failed apply lets the objects already in
//! flight finish and skips the rest. A cancel skips every object not yet sent.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::StreamExt as _;
use futures::stream;
use oxikube_domain::OxiError;

use super::detect::applied_change;
use super::item::Item;
use super::rehearse::{Rehearsal, applied_status};
use super::run::{APPLIES_AT_ONCE, Run, failed, failure};
use super::types::{ObjectResult, ObjectStatus};
use crate::apply::types::ApplyMethod;

/// What the objects already done say to the next ones.
struct Ledger<'a> {
    /// The results so far, by item index.
    results: &'a [Option<ObjectResult>],
    /// Definitions that were not `Established` in time.
    late: &'a HashSet<usize>,
    /// Set by the first failure when stop on error is on.
    stopped: &'a AtomicBool,
}

impl Run<'_> {
    /// Applies the plan; every object's result, in plan order.
    pub(super) async fn apply_all(&self) -> Vec<ObjectResult> {
        let rehearsals = self.rehearse_all().await;
        if self.stop_on_error
            && let Some(first) = rehearsals.iter().position(Rehearsal::blocks)
        {
            return self.refuse_all(&rehearsals, first);
        }
        let mut results: Vec<Option<ObjectResult>> = vec![None; self.items.len()];
        let mut late: HashSet<usize> = HashSet::new();
        let stopped = AtomicBool::new(false);
        // Definitions applied whose custom resources have not been sent yet.
        let mut unsettled: Vec<usize> = Vec::new();
        let mut start = 0;
        while start < self.items.len() {
            let rank = self.items[start].rank;
            let end = start
                + self.items[start..]
                    .iter()
                    .take_while(|item| item.rank == rank)
                    .count();
            let needed: Vec<&Item> = unsettled
                .iter()
                .filter(|&&crd| {
                    self.items[start..]
                        .iter()
                        .any(|item| item.depends_on.contains(&crd))
                })
                .map(|&crd| &self.items[crd])
                .collect();
            if !needed.is_empty() {
                late.extend(self.settle_crds(&needed).await);
                unsettled.retain(|crd| !needed.iter().any(|item| item.index == *crd));
            }
            let ledger = Ledger {
                results: &results,
                late: &late,
                stopped: &stopped,
            };
            let group: Vec<_> = self.items[start..end]
                .iter()
                .map(|item| self.apply_one(item, &rehearsals[item.index], &ledger))
                .collect();
            let done: Vec<ObjectResult> = stream::iter(group)
                .buffer_unordered(APPLIES_AT_ONCE)
                .collect()
                .await;
            for result in done {
                let applied = matches!(
                    result.status,
                    ObjectStatus::Created | ObjectStatus::Configured
                );
                if applied && self.items[result.index].is_crd() {
                    unsettled.push(result.index);
                }
                let index = result.index;
                results[index] = Some(result);
            }
            start = end;
        }
        if !unsettled.is_empty() {
            // Nothing in the plan needs them, but the cluster's kinds changed.
            self.refresh_discovery().await;
        }
        results.into_iter().flatten().collect()
    }

    /// Stop on error with a failed dry run: nothing is sent; the failures say why, the rest name
    /// the first.
    fn refuse_all(&self, rehearsals: &[Rehearsal], first: usize) -> Vec<ObjectResult> {
        let reason = format!(
            "not applied: {} failed its dry run and stop on error is set",
            self.items[first].label
        );
        self.items
            .iter()
            .zip(rehearsals)
            .map(|(item, rehearsal)| {
                let status = match rehearsal {
                    Rehearsal::Failed(status) => status.clone(),
                    _ => ObjectStatus::skipped(reason.clone()),
                };
                self.finish(item, status, None)
            })
            .collect()
    }

    async fn apply_one(
        &self,
        item: &Item,
        rehearsal: &Rehearsal,
        ledger: &Ledger<'_>,
    ) -> ObjectResult {
        let (status, method) = self.decide(item, rehearsal, ledger).await;
        if self.stop_on_error && matches!(status, ObjectStatus::Failed { .. }) {
            ledger.stopped.store(true, Ordering::SeqCst);
        }
        self.finish(item, status, method)
    }

    /// Sends `item`, or says why not.
    async fn decide(
        &self,
        item: &Item,
        rehearsal: &Rehearsal,
        ledger: &Ledger<'_>,
    ) -> (ObjectStatus, Option<ApplyMethod>) {
        if self.cx.is_cancelled() {
            return (ObjectStatus::skipped("cancelled: not sent"), None);
        }
        if ledger.stopped.load(Ordering::SeqCst) {
            let reason = "not sent: an earlier object failed and stop on error is set";
            return (ObjectStatus::skipped(reason), None);
        }
        for &needed in &item.depends_on {
            let label = &self.items[needed].label;
            if ledger.late.contains(&needed) {
                let reason =
                    format!("not sent: it needs {label}, which was not Established in time");
                return (ObjectStatus::skipped(reason), None);
            }
            let failed_before = ledger.results[needed]
                .as_ref()
                .is_some_and(|result| result.status.is_failure());
            if failed_before {
                let reason = format!("not sent: it needs {label}, which did not apply");
                return (ObjectStatus::skipped(reason), None);
            }
        }
        match rehearsal {
            Rehearsal::Failed(status) => (status.clone(), None),
            Rehearsal::Skipped(reason) => (ObjectStatus::skipped(reason.clone()), None),
            Rehearsal::Ready { .. } | Rehearsal::NoPreview | Rehearsal::Deferred => {
                self.send(item).await
            }
        }
    }

    /// The write itself, audited: the server rehearses it once more (a deferred object is
    /// validated here, now that what it needs exists), then it is applied, and the object read
    /// just before says what it did.
    async fn send(&self, item: &Item) -> (ObjectStatus, Option<ApplyMethod>) {
        let Ok(write) = &item.write else {
            return (failed("the object names no kind or name"), None);
        };
        let work = async {
            let before = write.live(self.reader).await?;
            let (method, after) = write.send(self.mutation.writer(), self.reader).await?;
            Ok::<_, OxiError>((applied_change(before.as_ref(), &after), method))
        };
        match self.audited(item, work).await {
            Ok(Ok((change, method))) => (applied_status(change), Some(method)),
            Ok(Err(error)) => (failure(&error), None),
            Err(status) => (status, None),
        }
    }
}
