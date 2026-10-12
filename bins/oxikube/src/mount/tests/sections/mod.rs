//! The section matrix on the real init path (E12-S20): every kind of
//! `oxikube_testkit::sections` checked at once against the app as it is mounted, over fakes.
//!
//! * [`registry`]: the kinds the sidebar lists are exactly the matrix's, in its order, each with
//!   its documented columns, its own row actions and its detail panel; a kind's declared actions
//!   agree with the actions its rows offer.
//! * [`tables`]: every kind's sidebar entry opens its table, which lists the kind's seeded object
//!   with the documented columns, and the object's detail opens and draws.
//! * [`guard`]: every mutating row action of every kind is refused on a read-only cluster (for
//!   the user and for an agent), asks for its declared confirmation tier, leaves an audit record,
//!   and has an MCP tool stub; every mutating command the app declares has a matrix entry.
//!
//! The same matrix runs against a kind cluster in `tests/kind_sections.rs`.

mod guard;
mod registry;
mod tables;

use std::sync::Arc;

use oxikube_app::{CommandBus, RowActions};
use oxikube_domain::ids::{Gvk, ResourceRef, Scope};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_domain::session::NamespaceSelection;
use oxikube_testkit::sections::{KindCase, Seed, catalog, seed_resource};
use oxikube_testkit::{FakeResourcePort, TestPorts};

use super::App;
use crate::app_state::AppState;

/// Where the fake suite seeds: one namespace, and a stem for cluster-scoped names.
pub(super) const SEED: Seed<'static> = Seed {
    namespace: "sections",
    stem: "matrix",
};

/// `case`'s kind as discovery serves it: every verb.
pub(super) fn served(case: &KindCase) -> ResourceKind {
    ResourceKind {
        gvk: case.gvk(),
        preferred: true,
        plural: case.plural.into(),
        singular: case.kind.to_ascii_lowercase(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: VerbSet::from_names([
            "create", "delete", "get", "list", "patch", "update", "watch",
        ]),
        namespaced: case.scope == Scope::Namespaced,
    }
}

/// The seeded object of `case`.
pub(super) fn target(case: &KindCase) -> ResourceRef {
    let cluster = TestPorts::cluster_id();
    let name = case.seed_name(&SEED);
    if case.namespaced() {
        ResourceRef::namespaced(cluster, case.gvk(), SEED.namespace, name)
    } else {
        ResourceRef::cluster_scoped(cluster, case.gvk(), name)
    }
}

/// A kind nobody registered: what every kind's rows offer (YAML, describe, edit, delete).
pub(super) fn unknown_kind() -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new("example.io", "v1", "Widget"),
        ..served(oxikube_testkit::sections::case("ConfigMap").expect("ConfigMaps"))
    }
}

impl App {
    /// The cluster's connection: discovery serves every kind of the matrix and holds one seeded
    /// object of each; then Enter connects it, with every namespace selected.
    pub(super) fn connect_sections(&mut self) {
        let ports = self.ports.connector.ports_for(&TestPorts::cluster_id());
        ports.discovery.set_kinds(catalog().map(served));
        for case in catalog() {
            ports.resources.insert(seed_resource(case, &SEED));
        }
        self.press("enter");
        self.tick();
        let state = self.vcx.update(|_, cx| AppState::global(cx));
        state
            .services()
            .sessions
            .set_namespace_selection(&TestPorts::cluster_id(), NamespaceSelection::All)
            .expect("the session is open");
        self.tick();
    }

    /// The connected cluster's resource port.
    pub(super) fn seeded_resources(&self) -> Arc<FakeResourcePort> {
        self.ports
            .connector
            .ports_for(&TestPorts::cluster_id())
            .resources
    }

    /// The app's bus.
    pub(super) fn bus(&mut self) -> CommandBus {
        let state = self.vcx.update(|_, cx| AppState::global(cx));
        state.command_bus().expect("the mount set the bus").clone()
    }

    /// The row actions as the app's tables get them: the mount's registry over its bus.
    pub(super) fn row_actions(&mut self) -> RowActions {
        RowActions::from_bus(&self.bus(), &crate::mount::logs::row_actions())
    }
}
