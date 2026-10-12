//! One object of a bulk command: its audit record and the work between `begin` and `finish`.

use std::future::Future;

use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::{ErrorKind, OxiError, OxiResult};

use super::report::BulkStatus;
use crate::command_bus::HandlerContext;
use crate::guard::Mutation;

/// What every row of one batch shares.
pub(super) struct Batch<'a> {
    /// The handler's context: the audit log, the initiator, who.
    pub cx: &'a HandlerContext,
    /// The guard's permit.
    pub mutation: &'a Mutation,
    /// The id every audit record of the batch carries.
    pub id: &'a str,
    /// The bulk command, named in each record (`via=selection::Delete`).
    pub via: CommandId,
}

impl Batch<'_> {
    /// Runs `work` for `target` between its own audit record's begin and finish.
    ///
    /// The record is written first and the object is touched only when it can be (fail closed: an
    /// audit log that cannot be written fails the row, and nothing is sent for it). `command` is
    /// the single-object command the row stands for (`pod::Delete`, `workload::Restart`);
    /// `detail` adds `key=value` words after `batch=<id> via=<command>`, never a value.
    /// `work` returns the row's status and, optionally, a note for the record (`rv=42`).
    pub(super) async fn row<Fut>(
        &self,
        command: CommandId,
        target: &ResourceRef,
        detail: Option<&str>,
        work: Fut,
    ) -> BulkStatus
    where
        Fut: Future<Output = (BulkStatus, Option<String>)>,
    {
        let Some(audit) = self.cx.audit() else {
            return BulkStatus::Failed("a bulk action needs the guard's audit log".to_owned());
        };
        if let Err(error) = audit.ensure_writable().await {
            return BulkStatus::Failed(format!("not sent: {}", error.message()));
        }
        let mut words = format!("batch={} via={}", self.id, self.via);
        if let Some(detail) = detail {
            words.push(' ');
            words.push_str(detail);
        }
        let attempt = audit.begin_described(
            self.cx.who(),
            self.cx.initiator(),
            command.as_str(),
            target.clone(),
            self.mutation.dry_run(),
            Some(&words),
        );
        let (status, note) = work.await;
        attempt.finish_noting(
            if status.is_failed() {
                AuditOutcome::Failed
            } else {
                AuditOutcome::Succeeded
            },
            note.as_deref(),
        );
        if let Err(error) = audit.flush().await {
            // The object was touched; the command's own flush reports the log as failed.
            tracing::warn!(error = %error, "could not flush the audit record of a bulk row");
        }
        status
    }
}

/// The row for a request the server answered with `error`: an object that is gone is skipped, the
/// rest failed.
pub(super) fn failed_or_gone(error: &OxiError) -> BulkStatus {
    if error.kind() == ErrorKind::NotFound {
        BulkStatus::Skipped("already gone".to_owned())
    } else {
        BulkStatus::Failed(error.message().to_owned())
    }
}

/// `Ok(status)` of a request, else the row for its error.
pub(super) fn settle<T>(result: OxiResult<T>, ok: impl FnOnce(T) -> BulkStatus) -> BulkStatus {
    match result {
        Ok(done) => ok(done),
        Err(error) => failed_or_gone(&error),
    }
}
