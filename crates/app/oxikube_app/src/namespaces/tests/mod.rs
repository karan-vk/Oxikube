//! Namespace create and delete through the real `CommandBus` and `MutationGuard` over
//! `oxikube_testkit` fakes (no runtime, no threads), and the input rules.

mod create;
mod delete;
mod flow;
mod input;

use std::collections::BTreeMap;

use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};

use crate::testing::{Harness, id};

/// The standard harness with the real namespace handlers.
fn harness() -> Harness {
    Harness::with_extra(|registry, _, _| super::register_commands(registry))
}

fn namespace_ref(cluster: &str, name: &str) -> ResourceRef {
    ResourceRef::cluster_scoped(id(cluster), Gvk::new("", "v1", "Namespace"), name)
}

fn create(cluster: &str, name: &str, labels: &[(&str, &str)]) -> Command {
    Command::NamespaceCreate {
        cluster: id(cluster),
        name: name.to_owned(),
        labels: labels
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
    }
}

fn delete(cluster: &str, name: &str, contents: &[(&str, u32)]) -> Command {
    Command::NamespaceDelete {
        target: namespace_ref(cluster, name),
        contents: contents
            .iter()
            .map(|(k, n)| ((*k).to_owned(), *n))
            .collect::<BTreeMap<_, _>>(),
    }
}
