//! The role picker of the binding dialog: only roles the binding may name are offered.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    div, px,
};
use oxikube_domain::rbac::{BindingKind, RoleKind};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::dialog::BindingDialog;
use super::draft::Field;
use crate::role::Level;

/// How many roles the picker lists under the filter.
const OFFERED: usize = 8;

/// `ClusterRole/view`: how a role is shown.
pub(super) fn role_label(kind: RoleKind, name: &str) -> String {
    format!("{}/{name}", kind.as_str())
}

impl BindingDialog {
    /// The roles listed under the filter: those the binding may name that match what is typed. A
    /// cluster whose roles could not be listed offers the typed name as a ClusterRole (and a Role
    /// for a RoleBinding) instead.
    pub fn offered_roles(&self, cx: &gpui::App) -> Vec<(RoleKind, String)> {
        let namespace = self.namespace.read(cx).value().to_string();
        let query = self.role_query.read(cx).value().trim().to_lowercase();
        let mut offered: Vec<(RoleKind, String)> = self
            .choices
            .offered_roles(self.role_namespace(&namespace))
            .into_iter()
            .filter(|(_, name)| query.is_empty() || name.to_lowercase().contains(&query))
            .take(OFFERED)
            .map(|(kind, name)| (kind, name.to_string()))
            .collect();
        let typed = self.role_query.read(cx).value().trim().to_owned();
        if !typed.is_empty() && self.roles_unreadable() {
            offered.push((RoleKind::ClusterRole, typed.clone()));
            if self.kind == BindingKind::RoleBinding {
                offered.push((RoleKind::Role, typed));
            }
        }
        offered
    }

    /// Whether listing the Roles or ClusterRoles was refused.
    pub(super) fn roles_unreadable(&self) -> bool {
        self.choices
            .notes
            .iter()
            .any(|note| note.starts_with("Roles ") || note.starts_with("ClusterRoles "))
    }

    pub(super) fn role_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let problem = self.problem_at(Field::Role, cx);
        let body = match (&self.role, self.role_open) {
            (Some(role), false) => {
                let label = role_label(role.kind, &role.name);
                h_flex()
                    .gap(u(px(8.)))
                    .items_center()
                    .child(
                        div()
                            .debug_selector(|| "binding-role-chosen".into())
                            .px(u(px(8.)))
                            .py(u(px(2.)))
                            .rounded(u(px(4.)))
                            .bg(colors.element)
                            .child(label),
                    )
                    .child(
                        div().debug_selector(|| "binding-role-change".into()).child(
                            Button::new("binding-role-change")
                                .xsmall()
                                .ghost()
                                .label("Change")
                                .on_click(cx.listener(|this, _, _, cx| this.change_role(cx))),
                        ),
                    )
                    .into_any_element()
            }
            _ => {
                let offered = self.offered_roles(cx);
                let none_note = offered.is_empty().then(|| {
                    div()
                        .debug_selector(|| "binding-role-none".into())
                        .text_size(u(px(12.)))
                        .text_color(colors.text_muted)
                        .child(match self.kind {
                            BindingKind::ClusterRoleBinding => {
                                "No ClusterRole matches (a ClusterRoleBinding can only grant a ClusterRole)"
                            }
                            BindingKind::RoleBinding => {
                                "No Role of this namespace or ClusterRole matches"
                            }
                        })
                });
                v_flex()
                    .gap(u(px(6.)))
                    .child(
                        div()
                            .debug_selector(|| "binding-role-query".into())
                            .child(Input::new(&self.role_query)),
                    )
                    .child(
                        h_flex()
                            .debug_selector(|| "binding-role-offers".into())
                            .flex_wrap()
                            .gap(u(px(4.)))
                            .children(offered.into_iter().enumerate().map(|(ix, (kind, name))| {
                                let label = role_label(kind, &name);
                                let id = label.clone();
                                div()
                                    .debug_selector(move || format!("binding-role-pick-{id}"))
                                    .child(
                                        Button::new(("binding-role-pick", ix))
                                            .xsmall()
                                            .label(label)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.pick_role(kind, &name, cx)
                                            })),
                                    )
                            })),
                    )
                    .children(none_note)
                    .into_any_element()
            }
        };
        v_flex()
            .gap(u(px(4.)))
            .child(
                div()
                    .text_size(u(px(12.)))
                    .text_color(colors.text_muted)
                    .child(match self.kind {
                        BindingKind::RoleBinding => {
                            "Role to grant (a Role of this namespace or a ClusterRole)"
                        }
                        BindingKind::ClusterRoleBinding => "ClusterRole to grant",
                    }),
            )
            .child(div().debug_selector(|| "binding-role".into()).child(body))
            .when_some(problem, |col, problem| {
                col.child(
                    div()
                        .debug_selector(|| "binding-role-problem".into())
                        .text_size(u(px(12.)))
                        .text_color(match problem.level {
                            Level::Error => colors.error,
                            Level::Warning => colors.warning,
                        })
                        .child(problem.message),
                )
            })
            .into_any_element()
    }
}
