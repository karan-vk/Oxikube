//! [`Distribution`]: which Kubernetes distribution a cluster runs, guessed from what it says about
//! itself, and [`detect_distribution`], the pure function that guesses.
//!
//! The evidence is the API server's version string (`v1.30.4-eks-a737599`, `v1.31.1+k3s1`), the
//! nodes' labels (`eks.amazonaws.com/nodegroup`, `node.openshift.io/os_id`), their `spec.providerID`
//! (`kind://docker/...`) and, for Docker Desktop, the node's name. Each rule in [`RULES`] names one
//! piece of evidence that only that distribution leaves; the first rule that matches wins, and
//! without a match the answer is [`Distribution::Unknown`]: a guess is never made from a cloud
//! alone (`aws://` is also kops or a hand-built cluster on EC2).

use oxikube_domain::Resource;

/// A Kubernetes distribution the overview can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Distribution {
    /// Amazon Elastic Kubernetes Service.
    Eks,
    /// Google Kubernetes Engine.
    Gke,
    /// Azure Kubernetes Service.
    Aks,
    /// Red Hat OpenShift (or OKD).
    OpenShift,
    /// k3s (and k3d, which runs it in Docker).
    K3s,
    /// Rancher's RKE2.
    Rke2,
    /// k0s.
    K0s,
    /// kind (Kubernetes in Docker).
    Kind,
    /// minikube.
    Minikube,
    /// MicroK8s.
    MicroK8s,
    /// Docker Desktop's built-in cluster.
    DockerDesktop,
    /// Nothing identified it: said as "unknown", never guessed.
    #[default]
    Unknown,
}

impl Distribution {
    /// The name shown on the cluster-info card.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Eks => "Amazon EKS",
            Self::Gke => "Google GKE",
            Self::Aks => "Azure AKS",
            Self::OpenShift => "OpenShift",
            Self::K3s => "k3s",
            Self::Rke2 => "RKE2",
            Self::K0s => "k0s",
            Self::Kind => "kind",
            Self::Minikube => "minikube",
            Self::MicroK8s => "MicroK8s",
            Self::DockerDesktop => "Docker Desktop",
            Self::Unknown => "unknown",
        }
    }

    /// Whether a rule identified the distribution.
    pub const fn is_known(self) -> bool {
        !matches!(self, Self::Unknown)
    }
}

/// One piece of evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// The server's `gitVersion` contains this text.
    Version(&'static str),
    /// A node has a label whose key starts with this text.
    LabelKey(&'static str),
    /// A node's `spec.providerID` starts with this text.
    ProviderId(&'static str),
    /// A node has exactly this name.
    NodeName(&'static str),
}

/// The rules, most specific first: OpenShift runs on every cloud, so its own label is looked at
/// before any cloud's; a managed cloud's version suffix before its labels.
pub const RULES: &[(Distribution, Evidence)] = &[
    (
        Distribution::OpenShift,
        Evidence::LabelKey("node.openshift.io/"),
    ),
    (Distribution::Eks, Evidence::Version("-eks-")),
    (Distribution::Gke, Evidence::Version("-gke.")),
    (Distribution::K3s, Evidence::Version("+k3s")),
    (Distribution::Rke2, Evidence::Version("+rke2")),
    (Distribution::K0s, Evidence::Version("+k0s")),
    (Distribution::Eks, Evidence::LabelKey("eks.amazonaws.com/")),
    (
        Distribution::Gke,
        Evidence::LabelKey("cloud.google.com/gke-"),
    ),
    (
        Distribution::Aks,
        Evidence::LabelKey("kubernetes.azure.com/"),
    ),
    (
        Distribution::Minikube,
        Evidence::LabelKey("minikube.k8s.io/"),
    ),
    (Distribution::MicroK8s, Evidence::LabelKey("microk8s.io/")),
    (Distribution::Kind, Evidence::ProviderId("kind://")),
    (Distribution::K3s, Evidence::ProviderId("k3s://")),
    (
        Distribution::DockerDesktop,
        Evidence::NodeName("docker-desktop"),
    ),
];

/// The distribution `git_version` (the API server's, when known) and `nodes` point to, by the
/// rules of [`RULES`]. Objects in `nodes` that are not Nodes are skipped.
pub fn detect_distribution<'a>(
    git_version: Option<&str>,
    nodes: impl IntoIterator<Item = &'a Resource> + Clone,
) -> Distribution {
    RULES
        .iter()
        .find(|(_, evidence)| matches(*evidence, git_version, nodes.clone()))
        .map_or(Distribution::Unknown, |(distribution, _)| *distribution)
}

fn matches<'a>(
    evidence: Evidence,
    git_version: Option<&str>,
    nodes: impl IntoIterator<Item = &'a Resource>,
) -> bool {
    if let Evidence::Version(text) = evidence {
        return git_version.is_some_and(|v| v.contains(text));
    }
    nodes
        .into_iter()
        .filter(|node| node.kind.is_node())
        .any(|node| match evidence {
            Evidence::LabelKey(prefix) => node.meta.labels.keys().any(|k| k.starts_with(prefix)),
            Evidence::ProviderId(prefix) => node
                .get_str("/spec/providerID")
                .is_some_and(|id| id.starts_with(prefix)),
            Evidence::NodeName(name) => &*node.meta.name == name,
            Evidence::Version(_) => false,
        })
}
