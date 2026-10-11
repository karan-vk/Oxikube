//! Following a reference to another object: the Service behind an Ingress or a route, the
//! Gateway a route attaches to, the GatewayClass of a Gateway.
//!
//! A link opens the target's detail (`resource::Open`). Core Services are `v1`; every other kind
//! (the Gateway API's) is resolved through discovery, which says which version the cluster
//! serves, so nothing here hard-codes `v1` or `v1alpha2`.

use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use gpui::Global;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef, Scope};
use oxikube_domain::kinds::ResourceKind;
use oxikube_domain::view::network::GATEWAY_API_GROUP;
use oxikube_ports::DiscoveryPort;

/// How long a discovery answer serves the panels opened after it: long enough that clicking
/// through routes costs one discovery, short enough that a Gateway API installed meanwhile shows.
const CACHE_TTL: Duration = Duration::from_secs(30);

/// The Gateway API kinds each connection's discovery last reported, shared by every panel so
/// opening a detail does not run discovery again.
#[derive(Default)]
pub(super) struct KindsCache {
    entries: HashMap<ClusterId, Cached>,
}

struct Cached {
    /// The discovery that answered: a reconnect's new port does not read the old one's answer.
    port: Weak<dyn DiscoveryPort>,
    at: Instant,
    kinds: NavKinds,
}

impl Global for KindsCache {}

impl KindsCache {
    /// What `port` said for `cluster` less than [`CACHE_TTL`] ago.
    pub(super) fn get(
        &self,
        cluster: &ClusterId,
        port: &Arc<dyn DiscoveryPort>,
    ) -> Option<NavKinds> {
        let cached = self.entries.get(cluster)?;
        let same = Weak::ptr_eq(&cached.port, &Arc::downgrade(port));
        (same && cached.at.elapsed() < CACHE_TTL).then(|| cached.kinds.clone())
    }

    /// Remembers `kinds`, what `port` said for `cluster`.
    pub(super) fn put(
        &mut self,
        cluster: ClusterId,
        port: &Arc<dyn DiscoveryPort>,
        kinds: NavKinds,
    ) {
        let entry = Cached {
            port: Arc::downgrade(port),
            at: Instant::now(),
            kinds,
        };
        self.entries.insert(cluster, entry);
    }
}

/// The kinds a cluster serves, as far as discovery has said.
#[derive(Clone, Default)]
pub(super) struct NavKinds {
    kinds: Option<Arc<[ResourceKind]>>,
}

impl NavKinds {
    /// Keeps the Gateway API kinds of `kinds`: the only ones a link needs discovery for.
    pub(super) fn new(mut kinds: Vec<ResourceKind>) -> Self {
        kinds.retain(|k| *k.gvk.group == *GATEWAY_API_GROUP);
        Self {
            kinds: Some(kinds.into()),
        }
    }

    /// Whether discovery has answered.
    pub(super) fn is_known(&self) -> bool {
        self.kinds.is_some()
    }

    /// The version and scope `group`/`kind` is served at, `None` until discovery answers or when
    /// the cluster does not serve it. A core Service needs no discovery.
    pub(super) fn resolve(&self, group: &str, kind: &str) -> Option<(Gvk, Scope)> {
        if group.is_empty() && kind == "Service" {
            return Some((Gvk::new("", "v1", "Service"), Scope::Namespaced));
        }
        let kinds = self.kinds.as_deref()?;
        let named = |k: &&ResourceKind| *k.gvk.group == *group && *k.gvk.kind == *kind;
        let found = kinds
            .iter()
            .filter(named)
            .find(|k| k.preferred)
            .or_else(|| kinds.iter().find(named))?;
        let scope = if found.namespaced {
            Scope::Namespaced
        } else {
            Scope::Cluster
        };
        Some((found.gvk.clone(), scope))
    }

    /// The object `group`/`kind` `name` of `namespace` (the panel's own namespace when `None`) as
    /// a reference to open.
    pub(super) fn target(
        &self,
        from: &ResourceRef,
        group: &str,
        kind: &str,
        namespace: Option<&str>,
        name: &str,
    ) -> Option<ResourceRef> {
        let (gvk, scope) = self.resolve(group, kind)?;
        Some(match scope {
            Scope::Cluster => ResourceRef::cluster_scoped(from.cluster.clone(), gvk, name),
            Scope::Namespaced => {
                let namespace = namespace.or(from.namespace())?;
                ResourceRef::namespaced(from.cluster.clone(), gvk, namespace, name)
            }
        })
    }
}
