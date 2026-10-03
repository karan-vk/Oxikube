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
//! Creating and writing records belongs to the `MutationGuard` pipeline, not
//! to this module.
//!
//! [`Initiator`] is declared here for now and re-exported by `domain::safety`
//! once the safety vocabulary lands (E02-S06); the path `oxikube_domain::audit::Initiator`
//! keeps working.

use std::sync::Arc;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::bounds::char_boundary_prefix;
use crate::ids::{ClusterId, ResourceRef};

/// Longest `who` or `cmd` string kept, in bytes. Longer values are cut on a char boundary.
pub const MAX_AUDIT_FIELD_BYTES: usize = 256;

/// Who asked for a mutation: the door it came through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Initiator {
    /// A direct UI gesture (button, context menu).
    Ui,
    /// A command from the palette, a keybinding or the command bus.
    Command,
    /// An agent tool call (ACP or MCP).
    Agent,
    /// An extension (plugin).
    Plugin,
}

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
        }
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
