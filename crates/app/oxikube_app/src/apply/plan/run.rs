//! [`Run`]: what every object of one `resource::ApplyPlan` dispatch shares: the guard's permit,
//! the cluster's ports, the batch id; and the two things each object does through it: its audit
//! record and its result report.

use std::future::Future;

use oxikube_domain::OxiError;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::command::CommandId;
use oxikube_ports::{ClockPort, DiscoveryPort, ResourceReader};

use super::item::Item;
use super::types::{ObjectResult, ObjectStatus};
use crate::apply::types::{ApplyMethod, FieldConflict};
use crate::command_bus::HandlerContext;
use crate::guard::Mutation;

/// How many objects of one rank are applied at once.
pub(super) const APPLIES_AT_ONCE: usize = 4;

/// How many dry runs are in flight at once.
pub(super) const DRY_RUNS_AT_ONCE: usize = 8;

/// One dispatch of `resource::ApplyPlan`.
pub(super) struct Run<'a> {
    /// The handler's context: the audit log, the initiator, the caller's progress.
    pub(super) cx: &'a HandlerContext,
    /// The guard's permit: the writer (re-checking read-only mode before every request).
    pub(super) mutation: &'a Mutation,
    /// The cluster's reader: the live objects, a CRD's conditions.
    pub(super) reader: &'a dyn ResourceReader,
    /// The cluster's discovery, refreshed after a CRD is established.
    pub(super) discovery: Option<&'a dyn DiscoveryPort>,
    /// The clock a CRD's `Established` poll sleeps on.
    pub(super) clock: &'a dyn ClockPort,
    /// The id every audit record of the plan carries.
    pub(super) batch: &'a str,
    /// The objects, in plan order.
    pub(super) items: &'a [Item],
    /// Stop at the first failure.
    pub(super) stop_on_error: bool,
}

impl Run<'_> {
    /// Whether this is the dry-run dispatch.
    pub(super) fn dry_run(&self) -> bool {
        self.mutation.dry_run()
    }

    /// The result of `item`, reported to the caller as it is decided.
    pub(super) fn finish(
        &self,
        item: &Item,
        status: ObjectStatus,
        method: Option<ApplyMethod>,
    ) -> ObjectResult {
        let result = ObjectResult {
            index: item.index,
            label: item.label.clone(),
            target: item.target().cloned(),
            dry_run: self.dry_run(),
            status,
            method,
        };
        if let Ok(report) = serde_json::to_value(&result) {
            self.cx.report(report);
        }
        result
    }

    /// Runs `work` on `item` between the begin and the finish of its own audit record: command
    /// `resource::Apply` (what the object's write is), `batch=<id> via=resource::ApplyPlan` and
    /// the field manager, never the object. The record is written first and the object touched
    /// only when it can be (fail closed): `Err` with the object's failure when the audit log
    /// cannot take the record, and `work` never runs.
    pub(super) async fn audited<T, E, Fut>(
        &self,
        item: &Item,
        work: Fut,
    ) -> Result<Result<T, E>, ObjectStatus>
    where
        Fut: Future<Output = Result<T, E>>,
    {
        let (Some(audit), Some(target), Ok(write)) =
            (self.cx.audit(), item.target(), item.write.as_ref())
        else {
            return Err(failed(
                "not sent: an apply plan needs the guard's audit log",
            ));
        };
        if let Err(error) = audit.ensure_writable().await {
            return Err(failed(format!("not sent: {}", error.message())));
        }
        let detail = format!(
            "batch={} via={} manager={} force={}",
            self.batch,
            CommandId::RESOURCE_APPLY_PLAN,
            write.manager,
            write.force
        );
        let attempt = audit.begin_described(
            self.cx.who(),
            self.cx.initiator(),
            CommandId::RESOURCE_APPLY.as_str(),
            target.clone(),
            self.dry_run(),
            Some(&detail),
        );
        let result = work.await;
        attempt.finish(match &result {
            Ok(_) => AuditOutcome::Succeeded,
            Err(_) => AuditOutcome::Failed,
        });
        if let Err(error) = audit.flush().await {
            // The object was touched; the command's own flush reports the log as failed.
            tracing::warn!(error = %error, "could not flush the audit record of a plan object");
        }
        Ok(result)
    }
}

/// A failure with `reason` alone.
pub(super) fn failed(reason: impl Into<String>) -> ObjectStatus {
    ObjectStatus::Failed {
        reason: reason.into(),
        fields: Vec::new(),
        conflicts: Vec::new(),
    }
}

/// The failure for `error`: its message, the fields a validation error names, and the managers of
/// the fields a server-side apply conflict names.
pub(super) fn failure(error: &OxiError) -> ObjectStatus {
    let fields = error
        .validation_details()
        .map(|details| {
            details
                .causes
                .iter()
                .map(|cause| cause.field.clone())
                .filter(|field| !field.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let conflicts = error
        .conflict_details()
        .map(|details| {
            details
                .causes
                .iter()
                .map(|cause| FieldConflict {
                    field: cause.field.clone(),
                    manager: cause.manager.clone(),
                    applied_at: None,
                })
                .collect()
        })
        .unwrap_or_default();
    ObjectStatus::Failed {
        reason: error.message().to_owned(),
        fields,
        conflicts,
    }
}
