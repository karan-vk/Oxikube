//! [`PolicyMatrix`]: normalised rules as k9s's policy table, one row per API group and
//! resource, one column per verb.

use std::collections::{BTreeSet, HashMap};

use super::normalise::{NonResourceRule, NormalisedRules};
use crate::access::{Access, AccessRule, resource_matches};
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

/// The API group a rule names to cover every group.
const ALL_GROUPS: &str = "*";

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
    ///
    /// A row only looks at the rules of its own API group and of `*`, so a policy of many groups
    /// (hundreds of aggregated CRD roles) costs about one pass, not rows times rules.
    pub fn from_rules(rules: &NormalisedRules) -> Self {
        let mut by_group: HashMap<&str, Vec<&AccessRule>> = HashMap::new();
        let mut keys: BTreeSet<(&str, &str)> = BTreeSet::new();
        for rule in rules.resource_rules() {
            for group in &rule.api_groups {
                by_group.entry(group.as_str()).or_default().push(rule);
                keys.extend(
                    rule.resources
                        .iter()
                        .map(|resource| (group.as_str(), resource.as_str())),
                );
            }
        }
        let wildcard: &[&AccessRule] = by_group.get(ALL_GROUPS).map_or(&[], Vec::as_slice);
        let rows = keys
            .into_iter()
            .map(|(group, resource)| {
                let own: &[&AccessRule] = if group == ALL_GROUPS {
                    &[]
                } else {
                    by_group.get(group).map_or(&[], Vec::as_slice)
                };
                row(own.iter().chain(wildcard).copied(), group, resource)
            })
            .collect();
        Self {
            rows,
            non_resource: rules.non_resource_rules().cloned().collect(),
        }
    }
}

/// The row of `group`/`resource`, answered by `candidates`: every rule that may cover it (the
/// others answer nothing anyway).
fn row<'a>(
    candidates: impl Iterator<Item = &'a AccessRule> + Clone,
    group: &str,
    resource: &str,
) -> MatrixRow {
    let cells = MATRIX_VERBS.map(|verb| {
        candidates
            .clone()
            .filter_map(|rule| rule.level_for(verb.as_str(), group, resource, None))
            .max()
            .unwrap_or(Access::Denied)
    });
    let mut extras = BTreeSet::new();
    let covering = candidates.filter(|rule| {
        rule.api_groups
            .iter()
            .any(|g| g == ALL_GROUPS || g == group)
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
