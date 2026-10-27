//! [`PolicyMatrix`]: normalised rules as k9s's policy table, one row per API group and
//! resource, one column per verb.

use std::collections::BTreeSet;

use super::normalise::{NonResourceRule, NormalisedRules};
use crate::access::{Access, resource_matches};
use crate::kinds::Verb;

/// The verb columns, in k9s's order: `GET LIST WATCH CREATE PATCH UPDATE DELETE DEL-LIST`.
/// Every other verb goes to [`MatrixRow::extras`] (`EXTRAS`).
pub const MATRIX_VERBS: [Verb; 8] = [
    Verb::Get,
    Verb::List,
    Verb::Watch,
    Verb::Create,
    Verb::Patch,
    Verb::Update,
    Verb::Delete,
    Verb::DeleteCollection,
];

/// The header of the column of extra verbs.
pub const EXTRAS_HEADER: &str = "EXTRAS";

/// The header of a verb column (`GET`, ..., `DEL-LIST`).
pub fn verb_header(verb: Verb) -> &'static str {
    match verb {
        Verb::Get => "GET",
        Verb::List => "LIST",
        Verb::Watch => "WATCH",
        Verb::Create => "CREATE",
        Verb::Patch => "PATCH",
        Verb::Update => "UPDATE",
        Verb::Delete => "DELETE",
        Verb::DeleteCollection => "DEL-LIST",
    }
}

/// One row: what the rules allow on one resource of one API group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixRow {
    /// The API group (`""` core, `*` any).
    pub api_group: String,
    /// The resource (`pods`, `pods/log`, `*`, `*/scale`).
    pub resource: String,
    /// One answer per [`MATRIX_VERBS`] column: [`Access::Granted`], [`Access::Restricted`]
    /// (named objects only) or [`Access::Denied`].
    pub cells: [Access; 8],
    /// Other verbs granted (`bind`, `escalate`, `use`, `impersonate`; `*` for "every verb"),
    /// sorted.
    pub extras: Vec<String>,
}

impl MatrixRow {
    /// The cell of `verb`'s column.
    pub fn cell(&self, verb: Verb) -> Access {
        MATRIX_VERBS
            .iter()
            .position(|column| *column == verb)
            .map_or(Access::Denied, |at| self.cells[at])
    }
}

/// k9s's policy view as data: rows sorted by API group then resource, plus the non-resource URL
/// rules beside them. A pure projection of [`NormalisedRules`], so a view only renders it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PolicyMatrix {
    /// Resource rows.
    pub rows: Vec<MatrixRow>,
    /// Non-resource URL rules, sorted by URL.
    pub non_resource: Vec<NonResourceRule>,
}

impl PolicyMatrix {
    /// Projects `rules`: one row per (API group, resource) any rule names, each cell answered by
    /// every rule that covers the row (so `*` in `apps` also fills the `deployments` row).
    pub fn from_rules(rules: &NormalisedRules) -> Self {
        let keys: BTreeSet<(&str, &str)> = rules
            .resource_rules()
            .flat_map(|rule| {
                rule.api_groups.iter().flat_map(|group| {
                    rule.resources
                        .iter()
                        .map(move |resource| (group.as_str(), resource.as_str()))
                })
            })
            .collect();
        let rows = keys
            .into_iter()
            .map(|(group, resource)| row(rules, group, resource))
            .collect();
        Self {
            rows,
            non_resource: rules.non_resource_rules().cloned().collect(),
        }
    }
}

fn row(rules: &NormalisedRules, group: &str, resource: &str) -> MatrixRow {
    let cells = MATRIX_VERBS.map(|verb| rules.level(verb.as_str(), group, resource, None));
    let mut extras = BTreeSet::new();
    let covering = rules.resource_rules().filter(|rule| {
        rule.api_groups.iter().any(|g| g == "*" || g == group)
            && rule.resources.iter().any(|r| resource_matches(r, resource))
    });
    for rule in covering {
        let others = rule
            .verbs
            .iter()
            .filter(|verb| Verb::parse(verb).is_none_or(|verb| !MATRIX_VERBS.contains(&verb)));
        extras.extend(others.cloned());
    }
    MatrixRow {
        api_group: group.to_owned(),
        resource: resource.to_owned(),
        cells,
        extras: extras.into_iter().collect(),
    }
}
