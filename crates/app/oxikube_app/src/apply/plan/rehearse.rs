//! The dry-run pass: every object of the plan rehearsed by the server before anything is written.
//!
//! An object whose dry run fails because it needs a Namespace or CustomResourceDefinition the plan
//! creates (a `depends_on` edge to an object that does not exist yet) is
//! [`DeferredDryRun`](ObjectStatus::DeferredDryRun), not an error: it is rehearsed again right
//! before it is applied. One that needs an object that cannot be applied is skipped.

use futures::StreamExt as _;
use futures::stream;
use oxikube_domain::{ErrorKind, OxiError};

use super::item::Item;
use super::run::{DRY_RUNS_AT_ONCE, Run, failed, failure};
use super::types::{ObjectResult, ObjectStatus};
use crate::apply::change::Change;
use crate::apply::types::ApplyMethod;

/// How one object's dry run ended.
#[derive(Debug, Clone)]
pub(super) enum Rehearsal {
    /// The server rehearsed it: applying it does `change`.
    Ready {
        /// Create, update or nothing.
        change: Change,
        /// How it would be sent.
        method: ApplyMethod,
    },
    /// The server cannot dry-run the kind: it is applied without a preview.
    NoPreview,
    /// It needs an object the plan creates first.
    Deferred,
    /// The server refused it: it is never sent. Always a [`ObjectStatus::Failed`].
    Failed(ObjectStatus),
    /// Not rehearsed: an object it needs cannot be applied, or the run was cancelled.
    Skipped(String),
}

impl Rehearsal {
    /// The dry run's answer, as a result.
    pub(super) fn status(&self) -> ObjectStatus {
        match self {
            Rehearsal::Ready { change, .. } => applied_status(*change),
            Rehearsal::NoPreview => ObjectStatus::NoPreviewAvailable,
            Rehearsal::Deferred => ObjectStatus::DeferredDryRun,
            Rehearsal::Failed(status) => status.clone(),
            Rehearsal::Skipped(reason) => ObjectStatus::skipped(reason.clone()),
        }
    }

    /// How it would be sent, when the server said.
    pub(super) fn method(&self) -> Option<ApplyMethod> {
        match self {
            Rehearsal::Ready { method, .. } => Some(*method),
            _ => None,
        }
    }

    /// Whether the object will not be applied: what needs it cannot be applied either.
    pub(super) fn blocks(&self) -> bool {
        matches!(self, Rehearsal::Failed(_) | Rehearsal::Skipped(_))
    }

    /// Whether the object will exist only once the plan applies it: what needs it waits.
    fn makes_new(&self) -> bool {
        matches!(
            self,
            Rehearsal::Deferred
                | Rehearsal::NoPreview
                | Rehearsal::Ready {
                    change: Change::Create,
                    ..
                }
        )
    }
}

/// The result word of `change`.
pub(super) fn applied_status(change: Change) -> ObjectStatus {
    match change {
        Change::Create => ObjectStatus::Created,
        Change::Update => ObjectStatus::Configured,
        Change::Unchanged => ObjectStatus::Unchanged,
    }
}

/// What the server said to one dry run, before the plan's edges are considered.
enum Answer {
    Done(Result<(ApplyMethod, Change), OxiError>),
    /// The cluster does not serve the kind (yet): the adapter's `Unsupported`, which is not a
    /// server that cannot dry-run it.
    Unserved(OxiError),
    /// The object cannot be written (it names no kind or name).
    Broken(String),
    /// The audit log could not take the record: not sent.
    NotAudited(ObjectStatus),
    Cancelled,
}

impl Run<'_> {
    /// The dry-run dispatch: every object rehearsed, audited and reported as soon as it is
    /// decided; the results, in plan order.
    pub(super) async fn dry_run_all(&self) -> Vec<ObjectResult> {
        let mut results = Vec::with_capacity(self.items.len());
        self.rehearse(true, |item, rehearsal| {
            results.push(self.finish(item, rehearsal.status(), rehearsal.method()));
        })
        .await;
        results
    }

    /// The real dispatch's rehearsal of every object before anything is written: neither audited
    /// nor reported (each object's write is).
    pub(super) async fn rehearse_all(&self) -> Vec<Rehearsal> {
        self.rehearse(false, |_, _| {}).await
    }

    /// The dry run of every object, a few at a time, in plan order; `each` sees every one as it
    /// is decided.
    async fn rehearse(
        &self,
        audit: bool,
        mut each: impl FnMut(&Item, &Rehearsal),
    ) -> Vec<Rehearsal> {
        // The futures are made up front: no closure is held across an await (which would make
        // the handler's future not `Send`).
        let dry_runs: Vec<_> = self
            .items
            .iter()
            .map(|item| self.rehearse_one(item, audit))
            .collect();
        let mut answers = stream::iter(dry_runs).buffered(DRY_RUNS_AT_ONCE);
        let mut out: Vec<Rehearsal> = Vec::with_capacity(self.items.len());
        while let Some(answer) = answers.next().await {
            let item = &self.items[out.len()];
            let rehearsal = self.judge(item, answer, &out);
            each(item, &rehearsal);
            out.push(rehearsal);
        }
        out
    }

    async fn rehearse_one(&self, item: &Item, audit: bool) -> Answer {
        if self.cx.is_cancelled() {
            return Answer::Cancelled;
        }
        let write = match &item.write {
            Ok(write) => write,
            Err(reason) => return Answer::Broken(reason.clone()),
        };
        let work = async {
            let live = write.live(self.reader).await?;
            let (method, object) = write
                .rehearse(self.mutation.writer(), live.as_ref())
                .await?;
            let live = live.map(|live| live.to_value());
            Ok((method, Change::between(live.as_ref(), &object.to_value())))
        };
        let answer: Result<(ApplyMethod, Change), OxiError> = if audit {
            match self.audited(item, work).await {
                Ok(answer) => answer,
                Err(status) => return Answer::NotAudited(status),
            }
        } else {
            work.await
        };
        match answer {
            Err(error) if error.kind() == ErrorKind::Unsupported && !self.serves(item).await => {
                Answer::Unserved(error)
            }
            answer => Answer::Done(answer),
        }
    }

    /// Whether the cluster's discovery serves the kind of `item`; `true` when it cannot say.
    async fn serves(&self, item: &Item) -> bool {
        let (Some(discovery), Some(target)) = (self.discovery, item.target()) else {
            return true;
        };
        !matches!(discovery.resolve(&target.gvk).await, Ok(None))
    }

    /// The rehearsal of `item` from its server's `answer` and the rehearsals before it.
    fn judge(&self, item: &Item, answer: Answer, earlier: &[Rehearsal]) -> Rehearsal {
        let blocked = item
            .depends_on
            .iter()
            .find(|&&needed| earlier.get(needed).is_some_and(Rehearsal::blocks));
        if let Some(&needed) = blocked {
            return Rehearsal::Skipped(format!(
                "not applied: it needs {}, which cannot be applied",
                self.items[needed].label
            ));
        }
        let awaited = item
            .depends_on
            .iter()
            .any(|&needed| earlier.get(needed).is_some_and(Rehearsal::makes_new));
        match answer {
            Answer::Cancelled => Rehearsal::Skipped("cancelled before its dry run".to_owned()),
            Answer::Broken(reason) => Rehearsal::Failed(failed(reason)),
            Answer::NotAudited(status) => Rehearsal::Failed(status),
            Answer::Done(Ok((method, change))) => Rehearsal::Ready { change, method },
            // The server cannot rehearse it before what it needs exists (a custom resource's
            // kind is not even served until its definition is).
            Answer::Done(Err(_)) | Answer::Unserved(_) if awaited => Rehearsal::Deferred,
            Answer::Done(Err(error)) if error.kind() == ErrorKind::Unsupported => {
                Rehearsal::NoPreview
            }
            Answer::Done(Err(error)) | Answer::Unserved(error) => {
                Rehearsal::Failed(failure(&error))
            }
        }
    }
}
