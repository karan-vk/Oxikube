//! [`AuditRecord`]: the append-only record of one mutation (ADR 0012).
//!
//! Every mutation passes through `MutationGuard`, which writes one record per
//! attempt: who asked, through which door ([`Initiator`]), which command, on
//! which object, whether it was a dry run, and how it ended. Records are
//! stored in SQLite (ADR 0010) and shown to agents, so field names are stable.
//!
//! # No bodies
//!
//! A record deliberately has no request or response body field: an applied
//! manifest or patch may carry Secret data, and the audit log is on disk
//! (non-negotiable 5). It names the target by [`ResourceRef`] and the command
//! by id, nothing more. The `who` and `cmd` strings are capped at
//! [`MAX_AUDIT_FIELD_BYTES`].
//!
//! A bulk operation writes one record per object sharing a `batch_id`; a refusal records
//! its [`AuditDenial`] beside the flat [`AuditOutcome`] so stored records stay readable.
//!
//! Creating and writing records belongs to the `MutationGuard` pipeline, not
//! to this module.
//!
//! [`Initiator`] is defined in [`crate::safety`] and re-exported here, so the
//! path `oxikube_domain::audit::Initiator` names the same type as
//! `oxikube_domain::safety::Initiator`.

use std::sync::Arc;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::bounds::char_boundary_prefix;
use crate::ids::{ClusterId, ResourceRef};
pub use crate::safety::Initiator;
use crate::safety::MutationVerb;

/// Longest `who` or `cmd` string kept, in bytes. Longer values are cut on a char boundary.
pub const MAX_AUDIT_FIELD_BYTES: usize = 256;

/// How a guarded mutation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    /// The cluster accepted the request (or, for a dry run, the server dry run passed).
    Succeeded,
    /// The request reached the cluster and failed.
    Failed,
    /// `MutationGuard` refused it (read-only cluster, policy) before any request.
    Denied,
    /// The user declined the confirmation, or the request was cancelled.
    Cancelled,
}

/// Why a guarded mutation was [`AuditOutcome::Denied`] or [`AuditOutcome::Cancelled`].
///
/// Kept beside the outcome rather than inside it so `AuditOutcome` keeps its flat serde form and
/// records already stored read back unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditDenial {
    /// The cluster is read-only.
    ReadOnly,
    /// The user declined the confirmation.
    UserCancelled,
    /// A policy refused it (agent permission, extension capability).
    Policy,
}

/// One audited mutation attempt.
///
/// See the [module docs](self); there is intentionally no body field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRecord {
    /// When the attempt ended.
    pub ts: Timestamp,
    /// The acting identity: the local user, or the agent or plugin name.
    pub who: Arc<str>,
    /// Which door the request came through.
    pub initiator: Initiator,
    /// The cluster the mutation targeted (equal to `target.cluster`).
    pub cluster: ClusterId,
    /// The command id, for example `pod::Delete`.
    pub cmd: Arc<str>,
    /// The object that was (or would have been) changed.
    pub target: ResourceRef,
    /// Whether this was a server-side dry run.
    pub dry_run: bool,
    /// How it ended.
    pub outcome: AuditOutcome,
    /// What else names the attempt, as short `key=value` words: the container and the program of
    /// a shell opened in a pod (`container=app program=bash`). Names only, never typed input or
    /// output (non-negotiable 5), capped at [`MAX_AUDIT_FIELD_BYTES`]. Absent for a mutation,
    /// which the command id and the target describe fully; records written before this field
    /// existed read back without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Arc<str>>,
    /// Groups the records of one bulk operation (a selection delete, a multi-object restart);
    /// absent for a single mutation and for records written before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<Arc<str>>,
    /// Why the attempt was denied or cancelled; absent for any other outcome and for records
    /// written before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denial: Option<AuditDenial>,
    /// What the mutation did to its target; absent for a record that did not come from a
    /// [`MutationIntent`](crate::safety::MutationIntent) and for records written before this field
    /// existed. The audit store indexes it for the viewer's verb filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verb: Option<MutationVerb>,
}

impl AuditRecord {
    /// Build a record. `cluster` is taken from `target`; `who` and `cmd` are cut
    /// to [`MAX_AUDIT_FIELD_BYTES`] on a char boundary.
    pub fn new(
        ts: Timestamp,
        who: &str,
        initiator: Initiator,
        cmd: &str,
        target: ResourceRef,
        dry_run: bool,
        outcome: AuditOutcome,
    ) -> Self {
        Self {
            ts,
            who: Arc::from(char_boundary_prefix(who, MAX_AUDIT_FIELD_BYTES)),
            initiator,
            cluster: target.cluster.clone(),
            cmd: Arc::from(char_boundary_prefix(cmd, MAX_AUDIT_FIELD_BYTES)),
            target,
            dry_run,
            outcome,
            detail: None,
            batch_id: None,
            denial: None,
            verb: None,
        }
    }

    /// Adds [`detail`](Self::detail), cut to [`MAX_AUDIT_FIELD_BYTES`] on a char boundary.
    #[must_use]
    pub fn with_detail(mut self, detail: &str) -> Self {
        self.detail = Some(Arc::from(char_boundary_prefix(
            detail,
            MAX_AUDIT_FIELD_BYTES,
        )));
        self
    }

    /// Adds [`batch_id`](Self::batch_id), cut to [`MAX_AUDIT_FIELD_BYTES`] on a char boundary.
    #[must_use]
    pub fn with_batch_id(mut self, batch_id: &str) -> Self {
        self.batch_id = Some(Arc::from(char_boundary_prefix(
            batch_id,
            MAX_AUDIT_FIELD_BYTES,
        )));
        self
    }

    /// Adds [`verb`](Self::verb).
    #[must_use]
    pub fn with_verb(mut self, verb: MutationVerb) -> Self {
        self.verb = Some(verb);
        self
    }

    /// Adds [`denial`](Self::denial).
    #[must_use]
    pub fn with_denial(mut self, denial: AuditDenial) -> Self {
        self.denial = Some(denial);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ContextName, Gvk};
    use proptest::prelude::*;

    fn target() -> ResourceRef {
        let cluster = ClusterId::new("kubeconfig", &ContextName::new("prod"));
        ResourceRef::namespaced(
            cluster,
            Gvk::from_api_version("v1", "Pod"),
            "default",
            "web-0",
        )
    }

    fn record(who: &str, cmd: &str) -> AuditRecord {
        AuditRecord::new(
            "2026-10-03T12:00:00Z".parse().unwrap(),
            who,
            Initiator::Agent,
            cmd,
            target(),
            true,
            AuditOutcome::Succeeded,
        )
    }

    #[test]
    fn cluster_follows_target() {
        let r = record("alice", "pod::Delete");
        assert_eq!(r.cluster, r.target.cluster);
    }

    #[test]
    fn serde_round_trip_every_initiator_and_outcome() {
        for initiator in [
            Initiator::Ui,
            Initiator::Command,
            Initiator::Agent,
            Initiator::Plugin,
        ] {
            for outcome in [
                AuditOutcome::Succeeded,
                AuditOutcome::Failed,
                AuditOutcome::Denied,
                AuditOutcome::Cancelled,
            ] {
                let mut r = record("alice", "pod::Delete");
                r.initiator = initiator;
                r.outcome = outcome;
                r.dry_run = outcome == AuditOutcome::Denied;
                let s = serde_json::to_string(&r).unwrap();
                assert_eq!(serde_json::from_str::<AuditRecord>(&s).unwrap(), r);
            }
        }
    }

    #[test]
    fn json_has_initiator_and_no_free_form_body() {
        let json = serde_json::to_value(record("alice", "workload::Scale")).unwrap();
        let obj = json.as_object().unwrap();
        assert_eq!(json["initiator"], "agent");
        assert_eq!(json["outcome"], "succeeded");
        assert_eq!(json["dry_run"], true);
        let mut keys: Vec<_> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "cluster",
                "cmd",
                "dry_run",
                "initiator",
                "outcome",
                "target",
                "ts",
                "who"
            ]
        );
        for banned in [
            "body", "request", "response", "patch", "manifest", "payload", "data",
        ] {
            assert!(!obj.contains_key(banned), "{banned}");
        }
    }

    #[test]
    fn detail_is_optional_bounded_and_old_records_still_read() {
        let plain = record("alice", "pod::Delete");
        assert!(plain.detail.is_none());
        let json = serde_json::to_value(&plain).unwrap();
        assert!(!json.as_object().unwrap().contains_key("detail"));
        let detailed = record("alice", "pod::Shell").with_detail("container=app program=bash");
        let text = serde_json::to_string(&detailed).unwrap();
        assert_eq!(
            serde_json::from_str::<AuditRecord>(&text).unwrap(),
            detailed
        );
        assert_eq!(
            detailed.detail.as_deref(),
            Some("container=app program=bash")
        );
        let long = record("alice", "pod::Shell").with_detail(&"é".repeat(1000));
        assert!(long.detail.unwrap().len() <= MAX_AUDIT_FIELD_BYTES);
    }

    #[test]
    fn batch_id_and_denial_are_optional_and_old_records_still_read() {
        let plain = serde_json::to_value(record("alice", "pod::Delete")).unwrap();
        assert!(plain.get("batch_id").is_none() && plain.get("denial").is_none());
        // A record stored before the fields existed reads back without them.
        let old: AuditRecord = serde_json::from_value(plain).unwrap();
        assert_eq!((old.batch_id, old.denial), (None, None));
        let full = record("alice", "selection::Delete")
            .with_batch_id("b-1")
            .with_denial(AuditDenial::ReadOnly);
        let text = serde_json::to_string(&full).unwrap();
        assert!(text.contains("\"denial\":\"read_only\""));
        assert_eq!(serde_json::from_str::<AuditRecord>(&text).unwrap(), full);
        let long = record("a", "c").with_batch_id(&"é".repeat(1000));
        assert!(long.batch_id.unwrap().len() <= MAX_AUDIT_FIELD_BYTES);
    }

    #[test]
    fn who_and_cmd_are_bounded() {
        let r = record(&"é".repeat(1000), &"c".repeat(1000));
        assert!(r.who.len() <= MAX_AUDIT_FIELD_BYTES);
        assert_eq!(r.cmd.len(), MAX_AUDIT_FIELD_BYTES);
    }

    proptest! {
        #[test]
        fn arbitrary_strings_never_panic(who in any::<String>(), cmd in any::<String>()) {
            let r = record(&who, &cmd);
            prop_assert!(r.who.len() <= MAX_AUDIT_FIELD_BYTES);
            prop_assert!(who.starts_with(&*r.who));
            prop_assert!(cmd.starts_with(&*r.cmd));
        }
    }
}
