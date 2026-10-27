//! Drawing the rule dialog.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use oxikube_domain::rbac::RoleKind;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{Cancel, DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};
use oxikube_workspace::modal::DIALOG_KEY_CONTEXT;

use super::dialog::{Load, RoleDialog};
use super::draft::{Field, RuleDraft};
use super::rule_card::problem_line;

/// What the dialog says about the rules of an aggregated ClusterRole.
pub const LOCKED_NOTE: &str = "This ClusterRole is aggregated: the control plane overwrites \
    its rules from the ClusterRoles its selectors pick, so they cannot be edited here. Change \
    the selectors with Edit as YAML, or edit the ClusterRoles it collects from.";

/// What the dialog says about where the change goes.
pub const SAVE_NOTE: &str = "Saving asks for a confirmation (an RBAC change can grant \
    privileges), the server checks it with a dry run first, and a cluster in read-only mode \
    refuses it.";

impl Render for RoleDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let edit = self.is_edit();
        let locked = self.rules_locked();
        let title = match (self.kind, edit) {
            (RoleKind::Role, false) => "New Role",
            (RoleKind::ClusterRole, false) => "New ClusterRole",
            (RoleKind::Role, true) => "Edit Role",
            (RoleKind::ClusterRole, true) => "Edit ClusterRole",
        };
        let ready = matches!(self.load, Load::Ready);
        let body = match &self.load {
            Load::Loading => div()
                .debug_selector(|| "role-dialog-loading".into())
                .text_color(colors.text_muted)
                .child("Reading the cluster's resources…")
                .into_any_element(),
            Load::Failed(why) => div()
                .debug_selector(|| "role-dialog-failed".into())
                .text_color(colors.error)
                .child(format!("Could not open the role: {why}"))
                .into_any_element(),
            Load::Ready => v_flex()
                .gap(u(px(10.)))
                .children((0..self.rules.len()).map(|ix| self.rule_card(ix, locked, cx)))
                .when(!locked, |list| {
                    list.child(
                        div().child(
                            Button::new("role-add-rule")
                                .label("Add rule")
                                .ghost()
                                .small()
                                .icon(Icon::new(IconName::Plus))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.add_rule(&RuleDraft::empty(), window, cx)
                                })),
                        ),
                    )
                })
                .children(self.problem_at(Field::Rules, cx).map(|p| {
                    problem_line(
                        &p,
                        "role-rules-problem".into(),
                        (colors.error, colors.warning),
                    )
                }))
                .into_any_element(),
        };
        let identity = h_flex()
            .gap(u(px(12.)))
            .child(
                self.labelled(
                    "Name",
                    "role-name",
                    Field::Name,
                    Input::new(&self.name).disabled(edit).into_any_element(),
                    cx,
                )
                .flex_1(),
            )
            .when(self.kind == RoleKind::Role, |row| {
                row.child(
                    self.labelled(
                        "Namespace",
                        "role-namespace",
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
            .id("role-dialog")
            .debug_selector(|| "role-dialog".into())
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
            .when(locked, |dialog| {
                dialog.child(
                    div()
                        .debug_selector(|| "role-locked-note".into())
                        .text_size(u(px(12.)))
                        .text_color(colors.warning)
                        .child(LOCKED_NOTE),
                )
            })
            .children(self.discovery_note.clone().map(|note| {
                div()
                    .debug_selector(|| "role-discovery-note".into())
                    .text_size(u(px(12.)))
                    .text_color(colors.warning)
                    .child(note)
            }))
            .child(
                div()
                    .id("role-dialog-body")
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
                        div().debug_selector(|| "role-yaml".into()).child(
                            Button::new("role-yaml")
                                .label("Edit as YAML")
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| this.edit_as_yaml(cx))),
                        ),
                    )
                    .child(
                        Button::new("role-cancel")
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    )
                    .child(
                        div().debug_selector(|| "role-save".into()).child(
                            Button::new("role-save")
                                .label(if edit { "Save" } else { "Create" })
                                .primary()
                                .disabled(!ready || locked)
                                .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                        ),
                    ),
            )
    }
}
