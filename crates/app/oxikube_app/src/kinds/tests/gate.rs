//! Discovery presence and the visibility hook.

use oxikube_domain::ids::{ClusterId, ContextName, Gvk, Scope};
use oxikube_domain::session::NamespaceSelection;
use oxikube_testkit::discovery_sets;

use super::built_ins;
use crate::kinds::{AllowAll, KindGate, KindRegistry, KindVisibility, ServedKinds, Shown};

fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("prod"))
}

fn registry() -> KindRegistry {
    let mut registry = KindRegistry::new();
    for meta in built_ins() {
        registry.register(meta).expect("registers");
    }
    registry
}

fn gateway() -> Gvk {
    Gvk::new("gateway.networking.k8s.io", "v1", "Gateway")
}

/// A hook that hides Secrets everywhere and Pods outside `dev`, and does not know about the rest.
struct Restricted;

impl KindVisibility for Restricted {
    fn is_listable(&self, _: &ClusterId, gvk: &Gvk, namespace: Option<&str>) -> Option<bool> {
        match &*gvk.kind {
            "Secret" => Some(false),
            "Pod" => Some(namespace == Some("dev")),
            _ => None,
        }
    }
}

#[test]
fn a_kind_the_cluster_does_not_serve_is_hidden() {
    let cluster = cluster();
    let selection = NamespaceSelection::All;
    let core = ServedKinds::from_kinds(&discovery_sets::core());
    let gate = KindGate {
        cluster: &cluster,
        served: Some(&core),
        visibility: &AllowAll,
        namespaces: &selection,
    };
    assert_eq!(gate.judge(&gateway(), Scope::Namespaced), Shown::NotServed);
    assert_eq!(
        gate.judge(&Gvk::new("apps", "v1", "Deployment"), Scope::Namespaced),
        Shown::Yes
    );
    let tree = registry().sidebar_tree(Some(&gate));
    let network: Vec<_> = tree
        .section("network")
        .expect("services")
        .entries
        .iter()
        .map(|e| e.meta.plural.to_string())
        .collect();
    assert_eq!(network, ["services"]);

    // The cluster installs the Gateway API: the entry appears.
    let with_gateway = ServedKinds::from_kinds(&discovery_sets::core_with_gateway_api());
    let gate = KindGate {
        served: Some(&with_gateway),
        ..gate
    };
    assert_eq!(gate.judge(&gateway(), Scope::Namespaced), Shown::Yes);
    let tree = registry().sidebar_tree(Some(&gate));
    assert_eq!(tree.section("network").expect("network").entries.len(), 2);
}

#[test]
fn before_discovery_answers_everything_counts_as_served() {
    let cluster = cluster();
    let selection = NamespaceSelection::All;
    let gate = KindGate {
        cluster: &cluster,
        served: None,
        visibility: &AllowAll,
        namespaces: &selection,
    };
    assert_eq!(gate.judge(&gateway(), Scope::Namespaced), Shown::Yes);
    assert_eq!(
        registry().sidebar_tree(Some(&gate)).len(),
        built_ins().len()
    );
}

#[test]
fn a_crd_only_discovery_hides_every_built_in_section() {
    let cluster = cluster();
    let selection = NamespaceSelection::All;
    let crds = ServedKinds::from_kinds(&discovery_sets::crds_only());
    let gate = KindGate {
        cluster: &cluster,
        served: Some(&crds),
        visibility: &AllowAll,
        namespaces: &selection,
    };
    assert!(registry().sidebar_tree(Some(&gate)).is_empty());
}

#[test]
fn the_default_hook_allows_everything() {
    let cluster = cluster();
    for gvk in built_ins().iter().map(|m| m.gvk.clone()) {
        assert_eq!(AllowAll.is_listable(&cluster, &gvk, None), None);
        assert_eq!(AllowAll.is_listable(&cluster, &gvk, Some("dev")), None);
    }
    let selection = NamespaceSelection::single("dev");
    let gate = KindGate {
        cluster: &cluster,
        served: None,
        visibility: &AllowAll,
        namespaces: &selection,
    };
    assert_eq!(
        registry().sidebar_tree(Some(&gate)).len(),
        built_ins().len()
    );
}

#[test]
fn a_stub_gate_hides_entries_per_namespace() {
    let cluster = cluster();
    let all = NamespaceSelection::All;
    let gate = KindGate {
        cluster: &cluster,
        served: None,
        visibility: &Restricted,
        namespaces: &all,
    };
    let secret = Gvk::new("", "v1", "Secret");
    let pod = Gvk::new("", "v1", "Pod");
    assert_eq!(gate.judge(&secret, Scope::Namespaced), Shown::NotListable);
    // Cluster-wide, pods are not listable; in `dev` they are.
    assert_eq!(gate.judge(&pod, Scope::Namespaced), Shown::NotListable);
    let dev_and_prod = NamespaceSelection::from_names(["prod", "dev"]);
    let gate_dev = KindGate {
        namespaces: &dev_and_prod,
        ..gate
    };
    assert_eq!(gate_dev.judge(&pod, Scope::Namespaced), Shown::Yes);
    let prod = NamespaceSelection::single("prod");
    let gate_prod = KindGate {
        namespaces: &prod,
        ..gate
    };
    assert_eq!(gate_prod.judge(&pod, Scope::Namespaced), Shown::NotListable);

    let tree = registry().sidebar_tree(Some(&gate));
    let config: Vec<_> = tree
        .section("config")
        .expect("configmaps")
        .entries
        .iter()
        .map(|e| e.meta.plural.to_string())
        .collect();
    assert_eq!(config, ["configmaps"], "secrets hidden by the hook");
}

#[test]
fn served_kinds_ignore_the_version() {
    let served = ServedKinds::from_kinds(&discovery_sets::core());
    assert!(served.serves(&Gvk::new("autoscaling", "v1", "HorizontalPodAutoscaler")));
    assert!(served.serves(&Gvk::new("autoscaling", "v2", "HorizontalPodAutoscaler")));
    assert!(!served.is_empty());
    assert!(served.len() >= 30);
}

/// Records the plural the gate hands the hook.
struct Plurals(parking_lot::Mutex<Vec<Option<String>>>);

impl KindVisibility for Plurals {
    fn is_listable_resource(
        &self,
        _: &ClusterId,
        _: &Gvk,
        plural: Option<&str>,
        _: Option<&str>,
    ) -> Option<bool> {
        self.0.lock().push(plural.map(str::to_owned));
        Some(false)
    }
}

#[test]
fn the_gate_hands_the_hook_the_plural_discovery_served() {
    let cluster = cluster();
    let selection = NamespaceSelection::All;
    let core = ServedKinds::from_kinds(&discovery_sets::core());
    let hook = Plurals(Default::default());
    let pods = Gvk::new("", "v1", "Pod");
    let gate = |served| KindGate {
        cluster: &cluster,
        served,
        visibility: &hook,
        namespaces: &selection,
    };

    assert_eq!(
        gate(Some(&core)).judge(&pods, Scope::Namespaced),
        Shown::NotListable
    );
    assert_eq!(
        gate(None).judge(&pods, Scope::Namespaced),
        Shown::NotListable
    );
    assert_eq!(*hook.0.lock(), [Some("pods".to_owned()), None]);
}
