//! Tests of the jump bar's grammar, planner, history and completion: a hand-made environment
//! (a stock cluster, cert-manager's CRDs, two more contexts) and a real `CommandBus` whose
//! handlers record what they were sent.

mod complete;
mod execute;
mod history;
mod parse;
mod plan;
mod print;

use std::collections::HashMap;
use std::sync::Arc;

use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_testkit::kinds::{cert_manager_kinds, core_kinds};

use super::{JumpContext, JumpEnv};
use crate::search::aliases::AliasTable;
use crate::session::namespaces::{NamespaceCatalog, NamespaceSource};

/// The cluster id of the context `name`.
pub(super) fn cluster(name: &str) -> ClusterId {
    ClusterId::new("tests", &ContextName::from(name))
}

/// Three contexts (`dev` and `prod-eu` connected, `staging` not), `dev` shown. `dev` serves the
/// stock types and cert-manager's CRDs; `prod-eu` the stock types only; the contexts that are not connected have discovered
/// nothing.
pub(super) struct Env {
    pub active: Option<ClusterId>,
    pub contexts: Vec<JumpContext>,
    pub tables: HashMap<ClusterId, AliasTable>,
    pub namespaces: HashMap<ClusterId, NamespaceCatalog>,
}

impl Env {
    pub fn new() -> Self {
        let context = |name: &str, connected| JumpContext {
            name: Arc::from(name),
            cluster: cluster(name),
            connected,
        };
        let stock = AliasTable::new();
        stock.set_discovered(&core_kinds());
        let dev = AliasTable::new();
        let mut kinds = core_kinds();
        kinds.extend(cert_manager_kinds());
        dev.set_discovered(&kinds);

        let listed = |names: &[&str]| NamespaceCatalog {
            names: names.iter().map(|n| (*n).to_owned()).collect(),
            source: NamespaceSource::Cluster,
        };
        Self {
            active: Some(cluster("dev")),
            contexts: vec![
                context("dev", true),
                context("prod-eu", true),
                context("prod-us", false),
                context("staging", false),
            ],
            tables: HashMap::from([
                (cluster("dev"), dev),
                (cluster("prod-eu"), stock),
                // Not connected: nothing discovered yet.
                (cluster("prod-us"), AliasTable::new()),
                (cluster("staging"), AliasTable::new()),
            ]),
            namespaces: HashMap::from([
                (
                    cluster("dev"),
                    listed(&["default", "kube-system", "monitoring", "web"]),
                ),
                (
                    cluster("prod-eu"),
                    listed(&["default", "kube-system", "payments", "web"]),
                ),
            ]),
        }
    }
}

impl JumpEnv for Env {
    fn active_cluster(&self) -> Option<ClusterId> {
        self.active.clone()
    }

    fn contexts(&self) -> &[JumpContext] {
        &self.contexts
    }

    fn aliases(&self, cluster: &ClusterId) -> AliasTable {
        self.tables.get(cluster).cloned().unwrap_or_default()
    }

    fn namespaces(&self, cluster: &ClusterId) -> Option<NamespaceCatalog> {
        self.namespaces.get(cluster).cloned()
    }
}
