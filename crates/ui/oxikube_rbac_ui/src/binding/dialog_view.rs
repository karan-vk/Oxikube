//! Drawing the binding dialog.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use oxikube_domain::rbac::BindingKind;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{Cancel, DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};
use oxikube_workspace::modal::DIALOG_KEY_CONTEXT;

use super::dialog::BindingDialog;
use super::draft::{Field, SubjectDraft};
use crate::role::{Level, Load};

/// What the dialog says when saving changes the role of an existing binding.
pub const RECREATE_NOTE: &str = "The API cannot change the role of a binding, so saving deletes \
    this binding and creates it again with the new role, as one confirmed step. If the new \
    binding is refused, the old one is put back.";

/// What the dialog says about where the change goes.
pub const SAVE_NOTE: &str = "Saving asks for a confirmation (a binding hands out privileges), \
    the server checks it with a dry run first, and a cluster in read-only mode refuses it.";

impl BindingDialog {
    /// `field` under a small muted `label`, with the problem of `at` below.
    fn labelled(
        &self,
        label: &'static str,
        id: &'static str,
        at: Field,
        field: AnyElement,
        cx: &mut Context<Self>,
    ) -> Div {
        let colors = cx.colors();
        v_flex()
            .gap(u(px(4.)))
            .child(
                div()
                    .text_size(u(px(12.)))
                    .text_color(colors.text_muted)
                    .child(label),
            )
            .child(div().debug_selector(move || id.into()).child(field))
            .children(self.problem_at(at, cx).map(|problem| {
                div()
                    .debug_selector(move || format!("{id}-problem"))
                    .text_size(u(px(12.)))
                    .text_color(match problem.level {
                        Level::Error => colors.error,
                        Level::Warning => colors.warning,
                    })
                    .child(problem.message)
            }))
    }

    /// The warnings about a grant to `cluster-admin` or to everybody, from the domain's table.
    fn escalation_notes(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = cx.colors();
        self.draft(cx)
            .escalations()
            .into_iter()
            .enumerate()
            .map(|(ix, escalation)| {
                div()
                    .debug_selector(move || format!("binding-escalation-{ix}"))
                    .text_size(u(px(12.)))
                    .text_color(colors.warning)
                    .child(escalation.message())
                    .into_any_element()
            })
            .collect()
    }
}

impl Render for BindingDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let edit = self.is_edit();
        let title = match (self.kind, edit) {
            (BindingKind::RoleBinding, false) => "New RoleBinding",
            (BindingKind::ClusterRoleBinding, false) => "New ClusterRoleBinding",
            (BindingKind::RoleBinding, true) => "Edit RoleBinding",
            (BindingKind::ClusterRoleBinding, true) => "Edit ClusterRoleBinding",
        };
        let ready = matches!(self.load, Load::Ready);
        let recreate = edit && self.recreates(cx);
        let body = match &self.load {
            Load::Loading => div()
                .debug_selector(|| "binding-dialog-loading".into())
                .text_color(colors.text_muted)
                .child("Reading the binding…")
                .into_any_element(),
            Load::Failed(why) => div()
                .debug_selector(|| "binding-dialog-failed".into())
                .text_color(colors.error)
                .child(format!("Could not open the binding: {why}"))
                .into_any_element(),
            Load::Ready => v_flex()
                .gap(u(px(10.)))
                .child(self.role_section(cx))
                .children(self.escalation_notes(cx))
                .when(recreate, |col| {
                    col.child(
                        div()
                            .debug_selector(|| "binding-recreate-note".into())
                            .text_size(u(px(12.)))
                            .text_color(colors.warning)
                            .child(RECREATE_NOTE),
                    )
                })
                .child(
                    div()
                        .text_size(u(px(12.)))
                        .text_color(colors.text_muted)
                        .child("Subjects"),
                )
                .children((0..self.subjects.len()).map(|ix| self.subject_card(ix, cx)))
                .child(
                    div().child(
                        Button::new("binding-add-subject")
                            .label("Add subject")
                            .ghost()
                            .small()
                            .icon(Icon::new(IconName::Plus))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_subject(&SubjectDraft::default(), window, cx)
                            })),
                    ),
                )
                .children(self.problem_at(Field::Subjects, cx).map(|problem| {
                    div()
                        .debug_selector(|| "binding-subjects-problem".into())
                        .text_size(u(px(12.)))
                        .text_color(colors.warning)
                        .child(problem.message)
                }))
                .into_any_element(),
        };
        let identity = h_flex()
            .gap(u(px(12.)))
            .child(
                self.labelled(
                    "Name",
                    "binding-name",
                    Field::Name,
                    Input::new(&self.name).disabled(edit).into_any_element(),
                    cx,
                )
                .flex_1(),
            )
            .when(self.kind == BindingKind::RoleBinding, |row| {
                row.child(
                    self.labelled(
                        "Namespace",
                        "binding-namespace",
                        Field::Namespace,
                        Input::new(&self.namespace)
                            .disabled(edit)
                            .into_any_element(),
                        cx,
                    )
                    .w(u(px(200.))),
                )
            });
        v_flex()
            .id("binding-dialog")
            .debug_selector(|| "binding-dialog".into())
            .key_context(DIALOG_KEY_CONTEXT)
            .track_focus(&self.focus)
            .tab_group()
            .w(u(px(780.)))
            .max_h(u(px(700.)))
            .gap(u(tokens.spacing.lg))
            .p(u(tokens.spacing.xxl))
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_lg()
            .on_action(cx.listener(|this, _: &Cancel, _, cx| this.cancel(cx)))
            .child(DialogHeader::new().child(DialogTitle::new().child(title)))
            .child(identity)
            .children(self.choices.notes.iter().enumerate().map(|(ix, note)| {
                div()
                    .debug_selector(move || format!("binding-note-{ix}"))
                    .text_size(u(px(12.)))
                    .text_color(colors.warning)
                    .child(note.clone())
            }))
            .child(
                div()
                    .id("binding-dialog-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            )
            .child(
                div()
                    .text_size(u(px(12.)))
                    .text_color(colors.text_muted)
                    .child(SAVE_NOTE),
            )
            .child(
                DialogFooter::new()
                    .child(
                        div().debug_selector(|| "binding-yaml".into()).child(
                            Button::new("binding-yaml")
                                .label("Edit as YAML")
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| this.edit_as_yaml(cx))),
                        ),
                    )
                    .child(
                        Button::new("binding-cancel")
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    )
                    .child(
                        div().debug_selector(|| "binding-save".into()).child(
                            Button::new("binding-save")
                                .label(if edit { "Save" } else { "Create" })
                                .primary()
                                .disabled(!ready)
                                .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                        ),
                    ),
            )
    }
}
