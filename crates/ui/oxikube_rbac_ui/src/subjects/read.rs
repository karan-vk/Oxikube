//! Reading the policy: the background passes behind the page, and their results.

use std::sync::Arc;

use gpui::Context;
use oxikube_app::rbac_subjects::{
    Inherit, PolicyHandle, SubjectList, SubjectMatrix, SubjectSummary, provenance, resolve_subject,
    scopes_of, subject_matrix, subject_name, summaries,
};
use oxikube_domain::rbac::{GrantScope, Subject};
use oxikube_runtime::notify_coalesced;

use super::model::{Page, Phase};
use super::view::SubjectsView;

impl SubjectsView {
    /// Reads the index again for the page that is shown, on the background executor. A request
    /// while one is running replaces it (the newest wins).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(handle) = self.policy.clone() else {
            return;
        };
        self.status = handle.status();
        self.phase = if self.status.pending > 0 {
            Phase::Loading {
                pending: self.status.pending,
            }
        } else {
            Phase::Ready
        };
        self.generation += 1;
        let generation = self.generation;
        match self.page {
            Page::Users | Page::Groups => {
                let list = if self.page == Page::Users {
                    SubjectList::Users
                } else {
                    SubjectList::Groups
                };
                self.read_task = Some(cx.spawn(async move |this, cx| {
                    let rows = cx
                        .background_executor()
                        .spawn(async move { handle.read(|index| summaries(index, list)) })
                        .await;
                    this.update(cx, |this, cx| this.listed(generation, rows, cx))
                        .ok();
                }));
            }
            Page::Subject => {
                let asked = self.asked.clone();
                let picked = self.picked_scope.clone();
                let inherit = self.inherit;
                self.read_task = Some(cx.spawn(async move |this, cx| {
                    let computed = cx
                        .background_executor()
                        .spawn(async move { compute(&handle, &asked, picked, inherit) })
                        .await;
                    this.update(cx, |this, cx| this.computed(generation, computed, cx))
                        .ok();
                }));
            }
        }
        cx.notify();
    }

    fn listed(&mut self, generation: u64, rows: Vec<SubjectSummary>, cx: &mut Context<Self>) {
        if generation != self.generation || self.page == Page::Subject {
            return;
        }
        let filter = self.filter.clone();
        let note = match (&self.phase, self.page) {
            (Phase::Loading { pending }, _) => {
                format!("Reading roles and bindings ({pending} of 4 lists pending)...")
            }
            (_, Page::Groups) => "No binding names a group.".to_owned(),
            _ => "No binding names a user or ServiceAccount.".to_owned(),
        };
        self.list.update_quiet(cx, |d| {
            d.set_rows(rows, &filter);
            d.set_empty_note(note);
        });
        self.list.refresh(cx);
        notify_coalesced(cx);
    }

    fn computed(&mut self, generation: u64, computed: Computed, cx: &mut Context<Self>) {
        if generation != self.generation || self.page != Page::Subject {
            return;
        }
        let Computed {
            subject,
            scopes,
            scope,
            matrix,
        } = computed;
        let note = self.empty_matrix_note(&subject, &matrix);
        let matrix = Arc::new(matrix);
        let selected_line = self.grid.read(cx, |d| {
            d.selected_row().and_then(|row| d.line(row).cloned())
        });
        let filter = self.filter.clone();
        let column = self.column;
        self.grid.update_quiet(cx, |d| {
            d.set_matrix(Some(matrix.clone()), &filter);
            d.set_note(note);
            d.set_column(Some(column));
            // The selection follows its line (the same group and resource) through a refresh.
            let row = selected_line.and_then(|old| {
                (0..d.len()).find(|&row| {
                    d.line(row)
                        .is_some_and(|l| l.api_group == old.api_group && l.resource == old.resource)
                })
            });
            d.select_row(row);
        });
        self.subject = Some(subject);
        self.scopes = scopes;
        self.scope = scope;
        self.matrix = Some(matrix);
        self.grid.refresh(cx);
        self.read_why(cx);
        notify_coalesced(cx);
    }

    fn empty_matrix_note(&self, subject: &Subject, matrix: &SubjectMatrix) -> String {
        if !matrix.is_empty() {
            return String::new();
        }
        if self.status.pending > 0 {
            return format!(
                "Reading roles and bindings ({} of 4 lists pending)...",
                self.status.pending
            );
        }
        let known = self.policy.as_ref().is_some_and(|policy| {
            policy.read(|index| index.subjects().iter().any(|s| same_identity(s, subject)))
        });
        if known {
            format!(
                "No rules reach {} {}. Its bindings may name roles that do not exist.",
                subject.kind(),
                subject_name(subject)
            )
        } else {
            format!(
                "No bindings found for \"{}\". Check the spelling, or pick a subject from Users or \
                 Groups.",
                self.asked
            )
        }
    }

    /// Reads the bindings behind the selected cell (off the UI thread).
    pub(super) fn read_why(&mut self, cx: &mut Context<Self>) {
        self.why_task = None;
        let (Some(policy), Some(subject)) = (self.policy.clone(), self.subject.clone()) else {
            self.why.clear();
            return;
        };
        let Some(line) = self
            .grid
            .read(cx, |d| d.selected_row().and_then(|r| d.line(r).cloned()))
        else {
            self.why.clear();
            return;
        };
        let (scope, inherit, column) = (self.scope.clone(), self.inherit, self.column);
        let generation = self.generation;
        self.why_task = Some(cx.spawn(async move |this, cx| {
            let why = cx
                .background_executor()
                .spawn(async move {
                    policy.read(|index| provenance(index, &subject, &scope, inherit, &line, column))
                })
                .await;
            this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.why = why;
                    cx.notify();
                }
            })
            .ok();
        }));
    }
}

/// What the background pass hands back for a subject page.
struct Computed {
    subject: Subject,
    scopes: Vec<GrantScope>,
    scope: GrantScope,
    matrix: SubjectMatrix,
}

/// Resolves the typed name, lists its scopes and projects its matrix.
fn compute(
    policy: &PolicyHandle,
    asked: &str,
    picked: Option<GrantScope>,
    inherit: Inherit,
) -> Computed {
    policy.read(|index| {
        let subject = resolve_subject(index, asked);
        let scopes = scopes_of(index, &subject, inherit);
        let (scope, matrix) = match picked {
            Some(scope) => {
                let matrix = subject_matrix(index, &subject, &scope, inherit);
                (scope, matrix)
            }
            None => {
                // Cluster-wide when the subject has grants there; otherwise the first namespace.
                let cluster = subject_matrix(index, &subject, &GrantScope::Cluster, inherit);
                match scopes.get(1) {
                    Some(first) if cluster.grants == 0 => (
                        first.clone(),
                        subject_matrix(index, &subject, first, inherit),
                    ),
                    _ => (GrantScope::Cluster, cluster),
                }
            }
        };
        Computed {
            subject,
            scopes,
            scope,
            matrix,
        }
    })
}

/// Whether two subjects are the same identity (`User system:serviceaccount:a:b` is the
/// ServiceAccount `a/b`).
fn same_identity(a: &Subject, b: &Subject) -> bool {
    let canonical = |s: &Subject| match s {
        Subject::User(name) => Subject::from_username(name),
        other => other.clone(),
    };
    canonical(a) == canonical(b)
}
