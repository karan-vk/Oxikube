//! [`SectionId`]: the sections of the pod panel.

/// The panel's sections, in the order they are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SectionId {
    /// Pod, ReplicaSet, Deployment...
    Owners,
    /// The node the pod runs on.
    Node,
    /// Each container's environment variables, ConfigMap and Secret references resolved.
    Environment,
    /// Each container's volume mounts.
    Mounts,
    /// Each container's liveness, readiness and startup probes.
    Probes,
    /// `spec.volumes`.
    Volumes,
    /// `spec.tolerations`.
    Tolerations,
    /// Node selector, affinity and topology spread.
    Affinity,
}

impl SectionId {
    /// Every section, top to bottom.
    pub const ALL: [SectionId; 8] = [
        SectionId::Owners,
        SectionId::Node,
        SectionId::Environment,
        SectionId::Mounts,
        SectionId::Probes,
        SectionId::Volumes,
        SectionId::Tolerations,
        SectionId::Affinity,
    ];

    /// The heading.
    pub const fn title(self) -> &'static str {
        match self {
            Self::Owners => "Owner chain",
            Self::Node => "Node",
            Self::Environment => "Environment",
            Self::Mounts => "Mounts",
            Self::Probes => "Probes",
            Self::Volumes => "Volumes",
            Self::Tolerations => "Tolerations",
            Self::Affinity => "Affinity",
        }
    }

    /// The name the section's test selectors carry.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Owners => "owners",
            Self::Node => "node",
            Self::Environment => "environment",
            Self::Mounts => "mounts",
            Self::Probes => "probes",
            Self::Volumes => "volumes",
            Self::Tolerations => "tolerations",
            Self::Affinity => "affinity",
        }
    }

    /// Whether the section starts open: the two short ones; the long lists are collapsed.
    pub const fn starts_open(self) -> bool {
        matches!(self, Self::Owners | Self::Node)
    }
}
