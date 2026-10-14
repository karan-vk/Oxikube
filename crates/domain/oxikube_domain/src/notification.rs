//! What the notification centre is told besides Kubernetes events (E14-S07): the kinds of
//! notification, and the typed inputs of the other sources with their dedupe keys.
//!
//! The emitting epics (port-forward manager E15, updater E24, agent permissions E27) map their
//! own state into these plain types and hand them to a `NotificationSourcePort`
//! (`oxikube_ports`); the app's `NotificationService` turns each into one notification, so all
//! sources share one list, one mute and one rate limit.
//!
//! # Dedupe keys
//!
//! A notification's *dedupe key* names what it is about; occurrences with the same key within
//! the dedupe window update one row (its count goes up) instead of adding rows. The formats:
//!
//! | Kind | Key | Built by |
//! |---|---|---|
//! | [`ClusterEvent`](NotificationKind::ClusterEvent) | `event/<cluster>/<object uid or kind/ns/name>/<reason>` | the rules (`oxikube_app`) |
//! | [`PortForward`](NotificationKind::PortForward) | `portforward:<cluster>:<forward-id>:dropped` | [`PortForwardChange::dedupe_key`] |
//! | [`UpdateAvailable`](NotificationKind::UpdateAvailable) | `update:<version>` | [`UpdateAvailable::dedupe_key`] |
//! | [`AgentPermission`](NotificationKind::AgentPermission) | `agent-permission:<session>:<request-id>` | [`agent_permission_key`] |
//!
//! Nothing here carries a secret: the text fields are redacted again by the centre.

use serde::{Deserialize, Serialize};

use crate::ids::{ClusterId, ResourceRef};

/// Where a notification comes from. The centre treats every kind uniformly (one list, one
/// rate limit) and lets the user switch a kind off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    /// A Warning event of a watched kind (the `EventService` feed).
    ClusterEvent,
    /// A port-forward dropped (E15).
    PortForward,
    /// A newer Oxikube version is available (E24).
    UpdateAvailable,
    /// An agent asked for permission and waits for the answer (E27).
    AgentPermission,
}

impl NotificationKind {
    /// Every kind, in a stable order.
    pub const ALL: [NotificationKind; 4] = [
        Self::ClusterEvent,
        Self::PortForward,
        Self::UpdateAvailable,
        Self::AgentPermission,
    ];

    /// The kind's name as settings and logs spell it (`cluster_event`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClusterEvent => "cluster_event",
            Self::PortForward => "port_forward",
            Self::UpdateAvailable => "update_available",
            Self::AgentPermission => "agent_permission",
        }
    }

    /// What the kind is called in a sentence.
    pub fn label(self) -> &'static str {
        match self {
            Self::ClusterEvent => "Cluster events",
            Self::PortForward => "Port-forward drops",
            Self::UpdateAvailable => "Updates",
            Self::AgentPermission => "Agent permission requests",
        }
    }

    /// Whether its toast stays until the notification is handled (read). Only an agent waiting
    /// for a permission answer does; the others go away with the normal toast timeout.
    pub fn sticky(self) -> bool {
        matches!(self, Self::AgentPermission)
    }
}

impl std::fmt::Display for NotificationKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a port-forward is in its life, as the port-forward manager (E15) reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ForwardPhase {
    /// Resolving the target and binding the listener.
    Starting,
    /// Listening and bridging connections.
    Active,
    /// The connection to the pod was lost; the manager is trying again.
    Reconnecting,
    /// The forward gave up or failed and is not running.
    Failed,
    /// The user (or the manager, on purpose) ended it.
    Stopped,
}

/// One state change of a port-forward, as E15's manager maps it. Not every change is a drop:
/// see [`is_drop`](Self::is_drop).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortForwardChange {
    /// The cluster the forward runs in.
    pub cluster: ClusterId,
    /// The manager's id of the forward; stable across reconnects.
    pub forward_id: String,
    /// What the forward is called for the user (`pod/api 8080:80`).
    pub label: String,
    /// The pod or service the forward targets, opened by a click on the notification.
    pub target: Option<ResourceRef>,
    /// The phase before.
    pub from: ForwardPhase,
    /// The phase now.
    pub to: ForwardPhase,
    /// Why, in one line (redacted by the centre), if known.
    pub reason: Option<String>,
}

impl PortForwardChange {
    /// Whether this change is a drop worth telling: an active forward that is reconnecting or
    /// failed, or one that gave up reconnecting. A stop, a start and a recovery are not.
    pub fn is_drop(&self) -> bool {
        use ForwardPhase::{Active, Failed, Reconnecting};
        matches!(
            (self.from, self.to),
            (Active, Reconnecting | Failed) | (Reconnecting, Failed)
        )
    }

    /// Whether this change brings a dropped forward back (`Reconnecting` or `Failed` to
    /// `Active`).
    pub fn is_recovery(&self) -> bool {
        use ForwardPhase::{Active, Failed, Reconnecting};
        matches!((self.from, self.to), (Reconnecting | Failed, Active))
    }

    /// `portforward:<cluster>:<forward-id>:dropped`: every drop of one forward shares it, so a
    /// flapping forward updates one row.
    pub fn dedupe_key(&self) -> String {
        format!("portforward:{}:{}:dropped", self.cluster, self.forward_id)
    }
}

/// A newer version is available (E24 maps its `UpdateInfo` into this).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateAvailable {
    /// The new version (`0.6.1`).
    pub version: String,
    /// Where to read about it or download it (an `https` link), if any.
    pub link: Option<String>,
    /// A one-line summary of the release, if any.
    pub summary: Option<String>,
}

impl UpdateAvailable {
    /// `update:<version>`: one row per version, whatever the number of checks that find it.
    pub fn dedupe_key(&self) -> String {
        format!("update:{}", self.version)
    }
}

/// An agent asked for permission to do something (E27 maps its request into this).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPermissionRequest {
    /// The agent session asking.
    pub session: String,
    /// The request's id within the session.
    pub request_id: String,
    /// What the agent is called (`Claude`).
    pub agent: String,
    /// What it wants to do, in one line (redacted by the centre).
    pub summary: String,
    /// The cluster the request concerns, if one.
    pub cluster: Option<ClusterId>,
    /// Whether the agent panel is focused: the user already sees the request, so nothing is
    /// raised.
    pub panel_focused: bool,
}

impl AgentPermissionRequest {
    /// `agent-permission:<session>:<request-id>`.
    pub fn dedupe_key(&self) -> String {
        agent_permission_key(&self.session, &self.request_id)
    }
}

/// `agent-permission:<session>:<request-id>`: the key of a permission request, for the answer
/// to find the notification it resolves.
pub fn agent_permission_key(session: &str, request_id: &str) -> String {
    format!("agent-permission:{session}:{request_id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ContextName;

    fn cluster() -> ClusterId {
        ClusterId::new("kubeconfig", &ContextName::new("kind"))
    }

    fn change(from: ForwardPhase, to: ForwardPhase) -> PortForwardChange {
        PortForwardChange {
            cluster: cluster(),
            forward_id: "fwd-1".into(),
            label: "pod/api 8080:80".into(),
            target: None,
            from,
            to,
            reason: None,
        }
    }

    #[test]
    fn only_losing_an_active_forward_or_giving_up_is_a_drop() {
        use ForwardPhase::*;
        let drops = [
            (Active, Reconnecting),
            (Active, Failed),
            (Reconnecting, Failed),
        ];
        for from in [Starting, Active, Reconnecting, Failed, Stopped] {
            for to in [Starting, Active, Reconnecting, Failed, Stopped] {
                assert_eq!(
                    change(from, to).is_drop(),
                    drops.contains(&(from, to)),
                    "{from:?} -> {to:?}"
                );
            }
        }
        assert!(change(Reconnecting, Active).is_recovery());
        assert!(!change(Starting, Active).is_recovery());
    }

    #[test]
    fn keys_follow_the_documented_formats() {
        assert_eq!(
            change(ForwardPhase::Active, ForwardPhase::Failed).dedupe_key(),
            format!("portforward:{}:fwd-1:dropped", cluster())
        );
        let update = UpdateAvailable {
            version: "0.6.1".into(),
            link: None,
            summary: None,
        };
        assert_eq!(update.dedupe_key(), "update:0.6.1");
        assert_eq!(agent_permission_key("s1", "r9"), "agent-permission:s1:r9");
    }

    #[test]
    fn only_agent_permission_is_sticky() {
        let sticky: Vec<_> = NotificationKind::ALL
            .into_iter()
            .filter(|k| k.sticky())
            .collect();
        assert_eq!(sticky, [NotificationKind::AgentPermission]);
        assert_eq!(NotificationKind::PortForward.to_string(), "port_forward");
    }
}
