//! [`SubjectMatrix`]: what the policy grants one subject, as k9s's per-verb table by API group,
//! and [`Provenance`], the bindings and roles behind one cell.

use std::fmt::Write as _;

use oxikube_domain::access::{Access, resource_matches};
use oxikube_domain::kinds::Verb;
use oxikube_domain::rbac::{
    BindingRef, EXTRAS_HEADER, GrantScope, MATRIX_VERBS, NonResourceRule, NormalisedRules,
    PolicyMatrix, RoleRef, Subject, verb_header,
};

use crate::access::{Grant, SubjectIndex};

/// Which bindings a subject's matrix includes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Inherit {
    /// Only the bindings that name the subject (k9s's view).
    #[default]
    Direct,
    /// Also the bindings of the groups the API server adds on its own: `system:authenticated`,
    /// and for a ServiceAccount `system:serviceaccounts` and `system:serviceaccounts:<ns>`
    /// ([`Subject::user_info`]). What a request made as the subject is allowed (`kubectl auth
    /// can-i --list --as`).
    ImplicitGroups,
}

/// What a [`MatrixLine`] is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// An API group and resource.
    Resource,
    /// A non-resource URL (`/healthz`, `/metrics/*`).
    Url,
}

/// One row of the matrix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixLine {
    /// Resource or non-resource URL.
    pub kind: LineKind,
    /// The API group (`""` core, `*` any); empty for a URL.
    pub api_group: String,
    /// The resource (`pods`, `*/scale`) or the URL.
    pub resource: String,
    /// One answer per [`MATRIX_VERBS`] column: granted, restricted to named objects, or denied.
    pub cells: [Access; 8],
    /// Other verbs granted (`bind`, `escalate`, `impersonate`, `*`), sorted.
    pub extras: Vec<String>,
}

impl MatrixLine {
    /// The group as the page shows it: `core` for the empty group, `-` for a URL.
    pub fn group_label(&self) -> &str {
        match (self.kind, self.api_group.as_str()) {
            (LineKind::Url, _) => "-",
            (LineKind::Resource, "") => "core",
            (LineKind::Resource, group) => group,
        }
    }

    /// The answer of `verb`'s column; [`Access::Denied`] for a verb that has none.
    pub fn cell(&self, verb: Verb) -> Access {
        MATRIX_VERBS
            .iter()
            .position(|column| *column == verb)
            .map_or(Access::Denied, |at| self.cells[at])
    }

    /// The text of the `column` cell: `yes`, `partial` or `-` (`EXTRAS`: the verbs).
    pub fn cell_text(&self, column: MatrixColumn) -> String {
        match column {
            MatrixColumn::Verb(verb) => access_word(self.cell(verb)).to_owned(),
            MatrixColumn::Extras if self.extras.is_empty() => "-".to_owned(),
            MatrixColumn::Extras => self.extras.join(","),
        }
    }
}

/// A column of the matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixColumn {
    /// One of the eight verb columns.
    Verb(Verb),
    /// The other verbs.
    Extras,
}

impl MatrixColumn {
    /// Every column, left to right: `GET .. DEL-LIST`, `EXTRAS`.
    pub fn all() -> impl Iterator<Item = MatrixColumn> {
        MATRIX_VERBS
            .into_iter()
            .map(MatrixColumn::Verb)
            .chain([MatrixColumn::Extras])
    }

    /// The column header.
    pub fn header(self) -> &'static str {
        match self {
            MatrixColumn::Verb(verb) => verb_header(verb),
            MatrixColumn::Extras => EXTRAS_HEADER,
        }
    }
}

fn access_word(access: Access) -> &'static str {
    match access {
        Access::Granted => "yes",
        Access::Restricted => "partial",
        Access::Denied | Access::Unknown => "-",
    }
}

/// Why one cell is what it is: a binding that gives a role that covers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    /// The RoleBinding or ClusterRoleBinding.
    pub binding: BindingRef,
    /// The role it names.
    pub role: RoleRef,
    /// Whether the role exists; a binding to a missing role grants nothing.
    pub role_found: bool,
    /// The subject entry of the binding that applies (a group when inherited).
    pub via: Subject,
    /// Where the grant holds.
    pub scope: GrantScope,
    /// Every object ([`Access::Granted`]) or only the named ones ([`Access::Restricted`]).
    pub access: Access,
    /// The `resourceNames` the grant is limited to, when restricted.
    pub names: Vec<String>,
}

/// What the policy grants one subject in one scope, projected for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectMatrix {
    /// Whose matrix.
    pub subject: Subject,
    /// Cluster-wide, or one namespace.
    pub scope: GrantScope,
    /// Whether implicit groups are included.
    pub inherit: Inherit,
    /// The rows: resources by API group and resource, then non-resource URLs.
    pub lines: Vec<MatrixLine>,
    /// How many bindings contribute in this scope.
    pub grants: usize,
    /// Bindings whose role does not exist, as `binding -> role`.
    pub dangling: Vec<String>,
}

impl SubjectMatrix {
    /// Whether no binding reaches the subject in this scope (the page says so instead of
    /// showing an empty table that looks like an error).
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The matrix as tab-separated text with a header row, for a ticket.
    pub fn to_tsv(&self) -> String {
        let mut out = String::from("API GROUP\tRESOURCE");
        for column in MatrixColumn::all() {
            let _ = write!(out, "\t{}", column.header());
        }
        out.push('\n');
        for line in &self.lines {
            let _ = write!(out, "{}\t{}", line.group_label(), line.resource);
            for column in MatrixColumn::all() {
                let _ = write!(out, "\t{}", line.cell_text(column));
            }
            out.push('\n');
        }
        out
    }
}

/// The grants that apply to `subject` in `scope`, per `inherit`. The index already expands the
/// subject to the groups the API server adds ([`Subject::user_info`]); [`Inherit::Direct`] keeps
/// the bindings that name the subject itself.
fn applicable(
    index: &SubjectIndex,
    subject: &Subject,
    scope: &GrantScope,
    inherit: Inherit,
) -> Vec<Grant> {
    let namespace = match scope {
        GrantScope::Cluster => None,
        GrantScope::Namespace(ns) => Some(&**ns),
    };
    index
        .grants_for(subject)
        .into_iter()
        .filter(|grant| grant.scope.covers(namespace))
        .filter(|grant| is_included(subject, grant, inherit))
        .collect()
}

fn is_included(subject: &Subject, grant: &Grant, inherit: Inherit) -> bool {
    match inherit {
        Inherit::ImplicitGroups => true,
        // A group's own bindings name that group; a user's name the user (either spelling).
        Inherit::Direct => match subject {
            Subject::Group(_) => grant.via == *subject,
            Subject::User(_) | Subject::ServiceAccount { .. } => {
                !matches!(grant.via, Subject::Group(_))
            }
        },
    }
}

/// The scopes `subject` holds grants in: the cluster first (always offered), then each namespace
/// with a RoleBinding that reaches it.
pub fn scopes_of(index: &SubjectIndex, subject: &Subject, inherit: Inherit) -> Vec<GrantScope> {
    let mut scopes: Vec<GrantScope> = index
        .grants_for(subject)
        .into_iter()
        .filter(|grant| is_included(subject, grant, inherit))
        .map(|grant| grant.scope)
        .chain([GrantScope::Cluster])
        .collect();
    scopes.sort();
    scopes.dedup();
    scopes
}

/// The matrix of `subject` in `scope`: the rules of every applicable grant, merged
/// ([`NormalisedRules::union`]) and projected ([`PolicyMatrix`]). A RoleBinding grants resources
/// only (URLs are never namespaced; the index already drops them from its grants).
pub fn subject_matrix(
    index: &SubjectIndex,
    subject: &Subject,
    scope: &GrantScope,
    inherit: Inherit,
) -> SubjectMatrix {
    let grants = applicable(index, subject, scope, inherit);
    let merged = NormalisedRules::union(grants.iter().map(|grant| &grant.rules));
    let projected = PolicyMatrix::from_rules(&merged);
    let mut lines: Vec<MatrixLine> = projected
        .rows
        .into_iter()
        .map(|row| MatrixLine {
            kind: LineKind::Resource,
            api_group: row.api_group,
            resource: row.resource,
            cells: row.cells,
            extras: row.extras,
        })
        .collect();
    lines.extend(projected.non_resource.iter().map(url_line));
    let dangling = grants
        .iter()
        .filter(|grant| !grant.role_found)
        .map(|grant| {
            format!(
                "{} -> {} {}",
                grant.binding,
                grant.role.kind.as_str(),
                grant.role.name
            )
        })
        .collect();
    SubjectMatrix {
        subject: subject.clone(),
        scope: scope.clone(),
        inherit,
        lines,
        grants: grants.len(),
        dangling,
    }
}

/// The bindings and roles behind one cell of `subject`'s matrix in `scope`. For an `EXTRAS`
/// cell, those that give a verb outside the eight columns.
pub fn provenance(
    index: &SubjectIndex,
    subject: &Subject,
    scope: &GrantScope,
    inherit: Inherit,
    line: &MatrixLine,
    column: MatrixColumn,
) -> Vec<Provenance> {
    applicable(index, subject, scope, inherit)
        .into_iter()
        .filter_map(|grant| {
            let (access, names) = cell_in(&grant.rules, line, column);
            (access != Access::Denied).then_some(Provenance {
                binding: grant.binding,
                role: grant.role,
                role_found: grant.role_found,
                via: grant.via,
                scope: grant.scope,
                access,
                names,
            })
        })
        .collect()
}

fn url_line(rule: &NonResourceRule) -> MatrixLine {
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
fn cell_in(
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
