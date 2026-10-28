//! Projecting one grant onto a matrix cell: non-resource URL lines and per-column answers.

use oxikube_domain::access::{Access, resource_matches};
use oxikube_domain::kinds::Verb;
use oxikube_domain::rbac::{MATRIX_VERBS, NonResourceRule, NormalisedRules};

use super::matrix::{LineKind, MatrixColumn, MatrixLine};

pub(super) fn url_line(rule: &NonResourceRule) -> MatrixLine {
    let any = rule.verbs.iter().any(|v| v == "*");
    let has = |verbs: &[&str]| any || rule.verbs.iter().any(|v| verbs.contains(&v.as_str()));
    let granted = |on: bool| if on { Access::Granted } else { Access::Denied };
    // The URL verbs are HTTP methods: map them onto the nearest columns.
    let cells = [
        granted(has(&["get"])),
        Access::Denied,
        Access::Denied,
        granted(has(&["post"])),
        granted(has(&["patch"])),
        granted(has(&["put"])),
        granted(has(&["delete"])),
        Access::Denied,
    ];
    let extras = rule
        .verbs
        .iter()
        .filter(|v| !matches!(v.as_str(), "get" | "post" | "patch" | "put" | "delete"))
        .cloned()
        .collect();
    MatrixLine {
        kind: LineKind::Url,
        api_group: String::new(),
        resource: rule.url.clone(),
        cells,
        extras,
    }
}

/// How `rules` answer one cell, and the `resourceNames` that limit it.
pub(super) fn cell_in(
    rules: &NormalisedRules,
    line: &MatrixLine,
    column: MatrixColumn,
) -> (Access, Vec<String>) {
    match line.kind {
        LineKind::Url => {
            let hit = rules.non_resource_rules().any(|rule| {
                rule.url == line.resource
                    && match column {
                        MatrixColumn::Verb(verb) => url_verbs(verb)
                            .iter()
                            .any(|v| rule.verbs.iter().any(|have| have == "*" || have == v)),
                        MatrixColumn::Extras => {
                            !line.extras.is_empty()
                                && rule.verbs.iter().any(|v| line.extras.contains(v))
                        }
                    }
            });
            (
                if hit { Access::Granted } else { Access::Denied },
                Vec::new(),
            )
        }
        LineKind::Resource => {
            let covering = rules.resource_rules().filter(|rule| {
                rule.api_groups
                    .iter()
                    .any(|g| g == "*" || *g == line.api_group)
                    && rule
                        .resources
                        .iter()
                        .any(|r| resource_matches(r, &line.resource))
            });
            match column {
                MatrixColumn::Verb(verb) => {
                    let access = rules.level(verb.as_str(), &line.api_group, &line.resource, None);
                    let names = if access == Access::Restricted {
                        let mut names: Vec<String> = covering
                            .filter(|rule| {
                                rule.verbs.iter().any(|v| v == "*" || v == verb.as_str())
                            })
                            .flat_map(|rule| rule.resource_names.clone())
                            .collect();
                        names.sort();
                        names.dedup();
                        names
                    } else {
                        Vec::new()
                    };
                    (access, names)
                }
                MatrixColumn::Extras => {
                    let extra = covering.into_iter().any(|rule| {
                        rule.verbs.iter().any(|v| {
                            Verb::parse(v).is_none_or(|verb| !MATRIX_VERBS.contains(&verb))
                        })
                    });
                    (
                        if extra {
                            Access::Granted
                        } else {
                            Access::Denied
                        },
                        Vec::new(),
                    )
                }
            }
        }
    }
}

/// The HTTP-method verbs a non-resource URL rule uses for a verb column.
fn url_verbs(verb: Verb) -> &'static [&'static str] {
    match verb {
        Verb::Get => &["get"],
        Verb::Create => &["post"],
        Verb::Patch => &["patch"],
        Verb::Update => &["put"],
        Verb::Delete => &["delete"],
        _ => &[],
    }
}
