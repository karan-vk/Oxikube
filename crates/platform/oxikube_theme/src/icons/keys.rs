//! The Kubernetes icon keys an icon theme may define, and the Lucide defaults behind them.
//!
//! Zed's icon-theme format knows files and folders. Oxikube's icons are kinds and statuses, so
//! they are looked up as ordinary `file_icons` keys with a prefix (ADR 0019). A Zed icon theme has
//! none of them and loads unchanged; whatever a theme leaves out falls back to Lucide.
//!
//! | Key | Overrides | Example |
//! |---|---|---|
//! | `kind:<group>/<Kind>` | the icon of one kind of one API group | `kind:argoproj.io/Application` |
//! | `kind:<Kind>` | the icon of a kind of any group (the core group's kinds have no group) | `kind:Pod` |
//! | `status:<Status>` | the icon of a status text as Oxikube shows it (phase, reason) | `status:CrashLoopBackOff` |
//! | `icon:<lucide-stem>` | one built-in Lucide icon everywhere it is drawn | `icon:box` |
//!
//! Zed's `chevron_icons` replace `icon:chevron-right` / `icon:chevron-down` and `directory_icons`
//! `icon:folder` / `icon:folder-open`; an explicit `icon:` key wins over them. A kind or status
//! without its own key falls back to its default Lucide icon, through any `icon:` override of
//! that icon (so `icon:server` also changes Node).

use oxikube_assets::IconName;

/// Prefix of a kind key (`kind:Pod`).
pub const KIND_KEY_PREFIX: &str = "kind:";
/// Prefix of a status key (`status:Running`).
pub const STATUS_KEY_PREFIX: &str = "status:";
/// Prefix of a built-in icon key (`icon:box`).
pub const ICON_KEY_PREFIX: &str = "icon:";

/// A key of `file_icons`, split by the convention above.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconKey<'a> {
    /// `kind:<group>/<Kind>` (`group` is `Some`) or `kind:<Kind>`.
    Kind {
        /// The API group, when the key names one.
        group: Option<&'a str>,
        /// The kind.
        kind: &'a str,
    },
    /// `status:<Status>`.
    Status(&'a str),
    /// `icon:<stem>` naming a built-in icon.
    Icon(IconName),
    /// Any other key: a Zed file-type key (`rust`, `yaml`), or an unknown `icon:` stem.
    File(&'a str),
}

impl<'a> IconKey<'a> {
    /// Splits `key`.
    pub fn parse(key: &'a str) -> Self {
        if let Some(rest) = key.strip_prefix(KIND_KEY_PREFIX) {
            return match rest.rsplit_once('/') {
                Some((group, kind)) => Self::Kind {
                    group: Some(group),
                    kind,
                },
                None => Self::Kind {
                    group: None,
                    kind: rest,
                },
            };
        }
        if let Some(status) = key.strip_prefix(STATUS_KEY_PREFIX) {
            return Self::Status(status);
        }
        match key
            .strip_prefix(ICON_KEY_PREFIX)
            .and_then(IconName::from_stem)
        {
            Some(icon) => Self::Icon(icon),
            None => Self::File(key),
        }
    }
}

/// The Lucide icon of a kind (`group` empty for the core group) when no icon theme says
/// otherwise. Unknown kinds (most custom resources) get [`IconName::FileCode`], a manifest.
pub fn default_kind_icon(group: &str, kind: &str) -> IconName {
    match (group, kind) {
        ("", "Pod") => IconName::Box,
        ("apps", "Deployment") => IconName::Boxes,
        ("apps", "ReplicaSet") => IconName::Copy,
        ("apps", "StatefulSet") => IconName::Database,
        ("apps", "DaemonSet") => IconName::Layers,
        ("batch", "Job") => IconName::Play,
        ("batch", "CronJob") => IconName::Clock,
        ("", "ConfigMap") => IconName::FileText,
        ("", "Secret") => IconName::KeyRound,
        ("", "ResourceQuota") => IconName::Gauge,
        ("", "LimitRange") => IconName::SlidersHorizontal,
        ("autoscaling", "HorizontalPodAutoscaler") => IconName::Scaling,
        ("policy", "PodDisruptionBudget") => IconName::Shield,
        ("scheduling.k8s.io", "PriorityClass") => IconName::ArrowUp,
        ("node.k8s.io", "RuntimeClass") => IconName::Cpu,
        ("coordination.k8s.io", "Lease") => IconName::Lock,
        ("", "Service") => IconName::Network,
        ("", "Endpoints") | ("discovery.k8s.io", "EndpointSlice") => IconName::Link,
        ("networking.k8s.io", "Ingress") => IconName::Globe,
        ("networking.k8s.io", "IngressClass") => IconName::Tag,
        ("networking.k8s.io", "NetworkPolicy") => IconName::ShieldCheck,
        ("", "PersistentVolumeClaim") => IconName::HardDrive,
        ("", "PersistentVolume") => IconName::Database,
        ("storage.k8s.io", "StorageClass") => IconName::Cloud,
        ("", "Node") => IconName::Server,
        ("", "Namespace") => IconName::Folder,
        ("", "Event") | ("events.k8s.io", "Event") => IconName::Bell,
        ("", "ServiceAccount") => IconName::User,
        ("rbac.authorization.k8s.io", "Role" | "ClusterRole") => IconName::ShieldCheck,
        ("rbac.authorization.k8s.io", "RoleBinding" | "ClusterRoleBinding") => IconName::Link,
        ("apiextensions.k8s.io", "CustomResourceDefinition") => IconName::Plug,
        ("argoproj.io", "Application" | "ApplicationSet") => IconName::GitBranch,
        _ => IconName::FileCode,
    }
}

/// The Lucide icon of a status text (a pod phase or reason, a workload or Argo CD state) when no
/// icon theme says otherwise: a check for healthy states, a clock for transitions, a cross for
/// failures, an alert for unknown ones and a plain circle for anything else.
pub fn default_status_icon(status: &str) -> IconName {
    match status {
        "Running" | "Ready" | "Succeeded" | "Completed" | "Complete" | "Active" | "Bound"
        | "Available" | "Healthy" | "Synced" | "Deployed" => IconName::CircleCheck,
        "Pending" | "ContainerCreating" | "PodInitializing" | "Terminating" | "Progressing"
        | "Waiting" | "Released" | "OutOfSync" | "PendingInstall" | "PendingUpgrade" => {
            IconName::Clock
        }
        "Failed" | "Error" | "CrashLoopBackOff" | "ImagePullBackOff" | "ErrImagePull"
        | "OOMKilled" | "Evicted" | "Lost" | "Degraded" | "NotReady" => IconName::CircleX,
        "Unknown" | "Missing" => IconName::CircleAlert,
        "Suspended" => IconName::Pause,
        _ => IconName::Circle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_split_by_prefix() {
        assert_eq!(
            IconKey::parse("kind:Pod"),
            IconKey::Kind {
                group: None,
                kind: "Pod"
            }
        );
        assert_eq!(
            IconKey::parse("kind:argoproj.io/Application"),
            IconKey::Kind {
                group: Some("argoproj.io"),
                kind: "Application"
            }
        );
        assert_eq!(IconKey::parse("status:Running"), IconKey::Status("Running"));
        assert_eq!(IconKey::parse("icon:box"), IconKey::Icon(IconName::Box));
        assert_eq!(
            IconKey::parse("icon:no-such"),
            IconKey::File("icon:no-such")
        );
        assert_eq!(IconKey::parse("rust"), IconKey::File("rust"));
    }

    #[test]
    fn kinds_default_to_lucide_icons() {
        assert_eq!(default_kind_icon("", "Pod"), IconName::Box);
        assert_eq!(default_kind_icon("apps", "Deployment"), IconName::Boxes);
        assert_eq!(default_kind_icon("", "Node"), IconName::Server);
        assert_eq!(default_kind_icon("", "Service"), IconName::Network);
        // The group matters: Knative's Service is not a core Service.
        assert_eq!(
            default_kind_icon("serving.knative.dev", "Service"),
            IconName::FileCode
        );
        assert_eq!(
            default_kind_icon("example.com", "Widget"),
            IconName::FileCode
        );
    }

    #[test]
    fn statuses_default_by_meaning() {
        assert_eq!(default_status_icon("Running"), IconName::CircleCheck);
        assert_eq!(default_status_icon("Pending"), IconName::Clock);
        assert_eq!(default_status_icon("Terminating"), IconName::Clock);
        assert_eq!(default_status_icon("CrashLoopBackOff"), IconName::CircleX);
        assert_eq!(default_status_icon("Unknown"), IconName::CircleAlert);
        assert_eq!(default_status_icon("Suspended"), IconName::Pause);
        assert_eq!(default_status_icon("Whatever"), IconName::Circle);
    }
}
