//! The matrix controls: scope, implicit groups, columns and copy.

use gpui::{ClipboardItem, Context};
use oxikube_app::rbac_subjects::{Inherit, MatrixColumn};
use oxikube_domain::command::Command;
use oxikube_domain::rbac::GrantScope;
use oxikube_workspace::Toast;

use super::model::Page;
use super::view::SubjectsView;

impl SubjectsView {
    /// Shows the matrix of `scope`.
    pub fn set_scope(&mut self, scope: GrantScope, cx: &mut Context<Self>) {
        if self.page != Page::Subject || scope == self.scope {
            return;
        }
        self.picked_scope = Some(scope);
        self.refresh(cx);
    }

    /// The next scope: cluster-wide, then each namespace, round.
    pub fn next_scope(&mut self, cx: &mut Context<Self>) {
        if self.scopes.len() < 2 {
            return;
        }
        let at = self
            .scopes
            .iter()
            .position(|s| *s == self.scope)
            .unwrap_or(0);
        let next = self.scopes[(at + 1) % self.scopes.len()].clone();
        self.set_scope(next, cx);
    }

    /// Includes the groups the API server adds on its own, or leaves them out.
    pub fn toggle_implicit_groups(&mut self, cx: &mut Context<Self>) {
        if self.page != Page::Subject {
            return;
        }
        self.inherit = match self.inherit {
            Inherit::Direct => Inherit::ImplicitGroups,
            Inherit::ImplicitGroups => Inherit::Direct,
        };
        self.refresh(cx);
    }

    /// Picks the verb column whose granting bindings the detail lists.
    pub fn set_column(&mut self, column: MatrixColumn, cx: &mut Context<Self>) {
        self.column = column;
        self.grid.update_quiet(cx, |d| d.set_column(Some(column)));
        self.grid.refresh(cx);
        self.read_why(cx);
        cx.notify();
    }

    /// Moves the picked column one step, stopping at the ends.
    pub fn step_column(&mut self, forward: bool, cx: &mut Context<Self>) {
        let all: Vec<MatrixColumn> = MatrixColumn::all().collect();
        let at = all.iter().position(|c| *c == self.column).unwrap_or(0);
        let next = if forward {
            (at + 1).min(all.len() - 1)
        } else {
            at.saturating_sub(1)
        };
        self.set_column(all[next], cx);
    }

    /// Copies the matrix as tab-separated text (`rbac::CopyMatrix`).
    pub fn copy_matrix(&mut self, cx: &mut Context<Self>) {
        let Some(matrix) = self.matrix.clone().filter(|m| !m.is_empty()) else {
            self.toast(Toast::info("There is no matrix to copy."), cx);
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(matrix.to_tsv()));
        self.toast(
            Toast::info(format!("Copied {} rows as text.", matrix.lines.len())),
            cx,
        );
    }

    /// Asks the bus to copy the matrix: the button and the key send the command.
    pub(super) fn request_copy(&mut self, cx: &mut Context<Self>) {
        self.deps.dispatcher.dispatch(
            Command::RbacCopyMatrix {
                cluster: self.cluster.clone(),
            },
            cx,
        );
    }

    fn toast(&self, toast: Toast, cx: &mut Context<Self>) {
        if let Some(workspace) = &self.workspace {
            workspace
                .update(cx, |workspace, cx| workspace.show_toast(toast, cx))
                .ok();
        }
    }
}
