//! Distribution detection, table-driven: each distribution from the evidence it leaves, the
//! order of the rules, and "unknown" where nothing identifies the cluster.

use oxikube_domain::Resource;
use oxikube_testkit::{node, pod};
use serde_json::json;

use crate::cluster_overview::{Distribution, detect_distribution};

fn with_provider(name: &str, provider_id: &str) -> Resource {
    let mut value = node().name(name).json();
    value["spec"]["providerID"] = json!(provider_id);
    Resource::from_json(value).expect("a node")
}

fn labelled(name: &str, key: &str, value: &str) -> Resource {
    node().name(name).label(key, value).build()
}

#[test]
fn each_distribution_is_named_from_its_own_evidence() {
    let plain = || node().name("worker").build();
    let cases: Vec<(&str, Option<&str>, Vec<Resource>, Distribution)> = vec![
        (
            "eks version",
            Some("v1.30.4-eks-a737599"),
            vec![plain()],
            Distribution::Eks,
        ),
        (
            "eks label",
            Some("v1.30.4"),
            vec![labelled(
                "ip-10-0-1-2",
                "eks.amazonaws.com/nodegroup",
                "ng-1",
            )],
            Distribution::Eks,
        ),
        (
            "gke version",
            Some("v1.31.1-gke.1146000"),
            vec![plain()],
            Distribution::Gke,
        ),
        (
            "gke label",
            None,
            vec![labelled(
                "gke-pool-1",
                "cloud.google.com/gke-nodepool",
                "pool-1",
            )],
            Distribution::Gke,
        ),
        (
            "aks label",
            Some("v1.29.7"),
            vec![labelled(
                "aks-np-1",
                "kubernetes.azure.com/cluster",
                "MC_rg",
            )],
            Distribution::Aks,
        ),
        (
            "k3s version",
            Some("v1.31.1+k3s1"),
            vec![plain()],
            Distribution::K3s,
        ),
        (
            "k3s provider",
            None,
            vec![with_provider("k3d-a-server-0", "k3s://k3d-a-server-0")],
            Distribution::K3s,
        ),
        (
            "rke2 version",
            Some("v1.30.5+rke2r1"),
            vec![plain()],
            Distribution::Rke2,
        ),
        (
            "k0s version",
            Some("v1.31.1+k0s"),
            vec![plain()],
            Distribution::K0s,
        ),
        (
            "kind provider",
            Some("v1.33.1"),
            vec![with_provider(
                "kind-control-plane",
                "kind://docker/kind/kind-control-plane",
            )],
            Distribution::Kind,
        ),
        (
            "openshift label",
            Some("v1.29.8+f10c92d"),
            vec![labelled("master-0", "node.openshift.io/os_id", "rhcos")],
            Distribution::OpenShift,
        ),
        (
            "minikube label",
            Some("v1.31.0"),
            vec![labelled("minikube", "minikube.k8s.io/name", "minikube")],
            Distribution::Minikube,
        ),
        (
            "microk8s label",
            Some("v1.31.0"),
            vec![labelled("host", "microk8s.io/cluster", "true")],
            Distribution::MicroK8s,
        ),
        (
            "docker desktop name",
            Some("v1.31.0"),
            vec![node().name("docker-desktop").build()],
            Distribution::DockerDesktop,
        ),
        (
            "unknown",
            Some("v1.33.1"),
            vec![plain()],
            Distribution::Unknown,
        ),
        (
            "a cloud alone is not a distribution",
            Some("v1.33.1"),
            vec![with_provider("ip-10-0-0-1", "aws:///eu-west-1a/i-0abc")],
            Distribution::Unknown,
        ),
        ("nothing at all", None, vec![], Distribution::Unknown),
    ];
    for (name, version, nodes, want) in cases {
        assert_eq!(detect_distribution(version, nodes.iter()), want, "{name}");
    }
}

#[test]
fn openshift_wins_over_the_cloud_it_runs_on() {
    let nodes = [node()
        .name("worker-0")
        .label("node.openshift.io/os_id", "rhcos")
        .label("kubernetes.azure.com/cluster", "aro")
        .build()];
    assert_eq!(
        detect_distribution(Some("v1.29.8"), nodes.iter()),
        Distribution::OpenShift
    );
}

#[test]
fn objects_that_are_not_nodes_are_ignored() {
    let pods = [pod()
        .name("docker-desktop")
        .label("eks.amazonaws.com/x", "1")
        .build()];
    assert_eq!(
        detect_distribution(None, pods.iter()),
        Distribution::Unknown
    );
}

#[test]
fn every_distribution_has_a_label_and_unknown_says_so() {
    assert_eq!(Distribution::Unknown.label(), "unknown");
    assert!(!Distribution::Unknown.is_known());
    assert_eq!(Distribution::Kind.label(), "kind");
    assert!(Distribution::Eks.is_known());
}
