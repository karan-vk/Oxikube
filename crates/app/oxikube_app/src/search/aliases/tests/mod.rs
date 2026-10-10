//! Alias table tests: layers, discovery, collisions, determinism and the follower, all over
//! `oxikube_testkit::kinds` fixtures and fakes.

mod collisions;
mod discovery;
mod follow;
mod layers;
mod props;

use oxikube_domain::AliasTarget;
use oxikube_domain::ids::Gvr;

use super::{AliasTable, Resolution};

/// A table over the stock cluster's types.
fn stock() -> AliasTable {
    let table = AliasTable::new();
    table.set_discovered(&oxikube_testkit::kinds::core_kinds());
    table
}

fn gvr(group: &str, version: &str, resource: &str) -> AliasTarget {
    AliasTarget::Gvr(Gvr::new(group, version, resource))
}

/// The one place `name` leads; panics when it is unknown or ambiguous.
fn exact(table: &AliasTable, name: &str) -> AliasTarget {
    match table.resolve(name) {
        Resolution::Exact(entry) => entry.target,
        other => panic!("`{name}` should be exact, got {other:?}"),
    }
}
