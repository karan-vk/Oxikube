//! Row action tests against `oxikube_testkit` fakes through the real `CommandBus` and
//! `MutationGuard` (see [`crate::testing::Harness`]).

mod flow;
mod guard;
mod resolve;

use oxikube_domain::command::{Command, Propagation};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::kinds::{ResourceKind, Verb, VerbSet};

use crate::testing::id;

pub(super) fn gvk(group: &str, kind: &str) -> Gvk {
    Gvk::new(group, "v1", kind)
}

pub(super) fn kind(group: &str, kind: &str, plural: &str, verbs: &[Verb]) -> ResourceKind {
    ResourceKind {
        gvk: gvk(group, kind),
        preferred: true,
        plural: plural.to_owned(),
        singular: kind.to_lowercase(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: verbs.iter().copied().collect::<VerbSet>(),
        namespaced: true,
    }
}

pub(super) const ALL_VERBS: [Verb; 8] = Verb::ALL;

pub(super) fn namespaced(
    cluster: &str,
    kind_group: &str,
    kind_name: &str,
    name: &str,
) -> ResourceRef {
    ResourceRef::namespaced(id(cluster), gvk(kind_group, kind_name), "default", name)
}

pub(super) fn cluster_scoped(cluster: &str, kind_name: &str, name: &str) -> ResourceRef {
    ResourceRef::cluster_scoped(id(cluster), gvk("", kind_name), name)
}

pub(super) fn delete(target: ResourceRef, propagation: Propagation) -> Command {
    Command::ResourceDelete {
        target,
        propagation,
    }
}
