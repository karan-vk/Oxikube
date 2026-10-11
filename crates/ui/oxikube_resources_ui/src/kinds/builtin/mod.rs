//! The built-in kinds, by sidebar section in Freelens' order: Nodes, Workloads, Config, Network,
//! Storage, Namespaces (then Custom Resources, from discovery). Each section module registers its
//! kinds from its own `init(cx)`; until a kind story gives a kind its own module, it has the
//! generic spec (the shared columns, the generic detail) and the actions E08/E09 already ship.
//!
//! Optional APIs (the Gateway API, VolumeSnapshots, the VerticalPodAutoscaler) are registered
//! like the others: the sidebar shows them only on clusters whose discovery serves them.
//!
//! Events (E14), Helm (E16) and Access Control (E17) are not here: those epics register their
//! own sections and kinds the same way.

mod config;
mod namespaces;
mod network;
mod nodes;
mod storage;
mod workloads;

use gpui::App;
use oxikube_app::{KindMeta, Section};
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{Gvk, Scope};

use super::spec::KindSpec;
use super::specs::KindSpecs;

/// One built-in kind as a section lists it.
#[derive(Clone, Copy, Debug)]
pub(super) struct Builtin {
    pub group: &'static str,
    pub version: &'static str,
    pub kind: &'static str,
    pub plural: &'static str,
    pub title: &'static str,
    pub scope: Scope,
}

/// A namespaced kind.
pub(super) const fn namespaced(
    group: &'static str,
    version: &'static str,
    kind: &'static str,
    plural: &'static str,
    title: &'static str,
) -> Builtin {
    Builtin {
        group,
        version,
        kind,
        plural,
        title,
        scope: Scope::Namespaced,
    }
}

/// A cluster-scoped kind.
pub(super) const fn cluster(
    group: &'static str,
    version: &'static str,
    kind: &'static str,
    plural: &'static str,
    title: &'static str,
) -> Builtin {
    Builtin {
        scope: Scope::Cluster,
        ..namespaced(group, version, kind, plural, title)
    }
}

/// The spec of `kind` in `section`, at `rank`, with the generic detail and `actions` of its own.
pub(super) fn spec(kind: &Builtin, section: Section, rank: u32, actions: &[CommandId]) -> KindSpec {
    let meta = KindMeta::new(
        Gvk::new(kind.group, kind.version, kind.kind),
        kind.plural,
        kind.title,
        kind.scope,
    )
    .in_section(section, rank)
    .with_actions(actions.iter().copied());
    KindSpec::new(meta)
}

/// Registers `kinds` in `section` in the order listed (ranks 10, 20, ...), each with the actions
/// `actions_of` gives it.
pub(super) fn register(
    cx: &mut App,
    section: Section,
    kinds: &[Builtin],
    actions_of: impl Fn(&Builtin) -> &'static [CommandId],
) {
    let specs = kinds
        .iter()
        .zip(1u32..)
        .map(|(kind, ix)| spec(kind, section, ix * 10, actions_of(kind)));
    if let Err(error) = KindSpecs::register_all(cx, specs) {
        tracing::error!(%error, section = section.id, "built-in kinds not all registered");
    }
}

/// No actions beyond the generic ones (open, YAML, describe, edit, delete).
pub(super) fn generic(_: &Builtin) -> &'static [CommandId] {
    &[]
}

/// Registers every built-in section's kinds and the kinds outside the sidebar.
pub fn init(cx: &mut App) {
    nodes::init(cx);
    workloads::init(cx);
    config::init(cx);
    network::init(cx);
    storage::init(cx);
    namespaces::init(cx);
    definitions(cx);
}

/// CustomResourceDefinitions: listed by the Custom Resources section's "Definitions" entry
/// (E07-S07), so no sidebar slot of their own; their action opens a CRD's resources.
fn definitions(cx: &mut App) {
    let meta = KindMeta::new(
        Gvk::new("apiextensions.k8s.io", "v1", "CustomResourceDefinition"),
        "customresourcedefinitions",
        "Definitions",
        Scope::Cluster,
    )
    .with_actions([CommandId::CRD_OPEN_RESOURCES]);
    if let Err(error) = KindSpecs::register(cx, KindSpec::new(meta)) {
        tracing::error!(%error, "the CRD kind was not registered");
    }
}
