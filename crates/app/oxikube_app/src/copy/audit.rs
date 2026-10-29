//! What the audit log says about a copy, on both clusters. Names and flags only, never an
//! object's fields (it may be a Secret, and the log is on disk).
//!
//! * The target's record is the guard's own, for `resource::CopyTo`: the copy's target, the
//!   [`detail`] (`copied_from=Secret shop/web source_cluster=<id> overwrite=false force=false`),
//!   and the context name and counts the handler adds.
//! * The source's record is [`SourceRead`]: the same command id on the source object, its detail
//!   naming where it went (`copied_to=...`), so the source cluster's log shows the read.

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::ResourceRef;

use crate::audit::{AuditAttempt, AuditLog};

/// "Secret shop/web", or "Namespace shop" for a cluster-scoped object: the kind and the place.
fn place(object: &ResourceRef) -> String {
    match object.namespace() {
        Some(namespace) => format!("{} {namespace}/{}", object.gvk.kind, object.name),
        None => format!("{} {}", object.gvk.kind, object.name),
    }
}

/// The detail of the guard's record for `command`; `None` for any other command.
pub(crate) fn detail(command: &Command) -> Option<String> {
    let Command::ResourceCopyTo {
        source,
        overwrite,
        force,
        ..
    } = command
    else {
        return None;
    };
    Some(format!(
        "copied_from={} source_cluster={} overwrite={overwrite} force={force}",
        place(source),
        source.cluster
    ))
}

/// The one-line confirmation of `command` on `context`; `None` for any other command.
pub(crate) fn summary(command: &Command, context: &str) -> Option<String> {
    let Command::ResourceCopyTo {
        source,
        target,
        overwrite,
        force,
    } = command
    else {
        return None;
    };
    let mut text = format!(
        "Copy {} to {} on {context}. The object is read from the source, stripped of what the \
         server owns and server-side applied here after a dry run.",
        place(source),
        place(target)
    );
    if *overwrite {
        text.push_str(" An object that exists is overwritten.");
    }
    if *force {
        text.push_str(" Fields other managers own are taken over (force).");
    }
    Some(text)
}

/// The record of reading the source object, written to the source cluster's log.
#[must_use = "finish the read with its outcome"]
pub(super) struct SourceRead<'a> {
    attempt: AuditAttempt<'a>,
    audit: &'a AuditLog,
}

impl<'a> SourceRead<'a> {
    /// Opens the record of reading `source` for a copy to `target`. The guard's own record of
    /// the copy is on the target cluster; this one is on the source's.
    pub(super) fn begin(
        audit: &'a AuditLog,
        who: &str,
        initiator: Initiator,
        source: &ResourceRef,
        target: &ResourceRef,
        dry_run: bool,
    ) -> Self {
        let detail = format!(
            "copied_to={} target_cluster={} read_only_access=true",
            place(target),
            target.cluster
        );
        let attempt = audit.begin_described(
            who,
            initiator,
            CommandId::RESOURCE_COPY_TO.as_str(),
            source.clone(),
            dry_run,
            Some(&detail),
        );
        Self { attempt, audit }
    }

    /// Records how the read ended and writes the record out.
    pub(super) async fn finish(self, succeeded: bool) {
        self.attempt.finish(if succeeded {
            AuditOutcome::Succeeded
        } else {
            AuditOutcome::Failed
        });
        if let Err(error) = self.audit.flush().await {
            tracing::warn!(error = %error, "could not flush the audit record of a copy's source read");
        }
    }
}
