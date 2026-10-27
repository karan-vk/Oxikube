//! Drawing the bound-roles section: a virtualised list of the roles the account holds.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    div, px, uniform_list,
};
use oxikube_domain::ids::ClusterId;
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Tokens, u};

use super::model::{BoundRole, BoundRoles};
use super::panel::ServiceAccountPanel;
use super::render::{heading, link, note};

/// The height of a bound-roles row, at 100 % zoom: uniform, as the virtualised list needs.
const ROW_HEIGHT: f32 = 26.;

/// How many bound roles show before the list scrolls.
const VISIBLE_ROWS: usize = 8;

impl ServiceAccountPanel {
    /// The roles bound to the account: a loading row, a note when nothing is bound, a note on
    /// the kinds the user may not list, and the rows.
    pub(super) fn bound_roles(
        &self,
        tokens: &Tokens,
        cluster: &ClusterId,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let count = match &self.bound {
            BoundRoles::Loading => None,
            BoundRoles::Known { roles, .. } => Some(roles.len()),
        };
        let mut section = v_flex()
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.sm))
            .gap(u(tokens.spacing.sm))
            .child(heading(tokens, "sa-bound", "Bound roles", count));
        match &self.bound {
            BoundRoles::Loading => {
                section = section.child(
                    div()
                        .debug_selector(|| "sa-bound-loading".to_owned())
                        .text_size(u(tokens.font.small))
                        .text_color(tokens.colors.text_muted)
                        .child("Reading roles and bindings…"),
                );
            }
            BoundRoles::Known { roles, missing } => {
                for (kind, why) in missing {
                    section = section.child(
                        div()
                            .debug_selector(move || format!("sa-bound-missing-{}", kind.as_str()))
                            .text_size(u(tokens.font.small))
                            .text_color(tokens.colors.warning)
                            .child(format!(
                                "{}s cannot be listed, so roles they grant are not shown: {why}",
                                kind.as_str()
                            )),
                    );
                }
                if roles.is_empty() {
                    let text = if missing.is_empty() {
                        "No role is bound to this account (system groups it belongs to included)."
                    } else {
                        "No bound role found in the kinds that can be listed."
                    };
                    section = section
                        .child(note(tokens, text).debug_selector(|| "sa-bound-none".to_owned()));
                } else {
                    section = section.child(self.bound_list(roles.len(), cluster, cx));
                }
            }
        }
        section.into_any_element()
    }

    fn bound_list(&self, total: usize, cluster: &ClusterId, cx: &mut Context<Self>) -> AnyElement {
        let cluster = cluster.clone();
        div()
            .debug_selector(|| "sa-bound-list".to_owned())
            .h(u(px(ROW_HEIGHT * total.min(VISIBLE_ROWS) as f32)))
            .w_full()
            .child(
                uniform_list(
                    "sa-bound-rows",
                    total,
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        let tokens = cx.tokens();
                        let BoundRoles::Known { roles, .. } = &this.bound else {
                            return Vec::new();
                        };
                        let rows: Vec<(usize, BoundRole)> = range
                            .filter_map(|ix| Some((ix, roles.get(ix)?.clone())))
                            .collect();
                        rows.into_iter()
                            .map(|(ix, role)| bound_row(&tokens, ix, &role, &cluster, cx))
                            .collect()
                    }),
                )
                .size_full(),
            )
            .into_any_element()
    }
}

/// One bound role: where it holds, the role, the binding, and the group it came through.
fn bound_row(
    tokens: &Tokens,
    ix: usize,
    bound: &BoundRole,
    cluster: &ClusterId,
    cx: &mut Context<ServiceAccountPanel>,
) -> AnyElement {
    let colors = tokens.colors;
    let (role_target, binding_target) = (bound.role_target(cluster), bound.binding_target(cluster));
    h_flex()
        .id(("sa-bound-row", ix))
        .debug_selector(move || format!("sa-bound-row-{ix}"))
        .h(u(px(ROW_HEIGHT)))
        .gap(u(tokens.spacing.md))
        .items_center()
        .text_size(u(tokens.font.small))
        .child(
            div()
                .debug_selector(move || format!("sa-bound-scope-{ix}"))
                .flex_none()
                .w(u(px(150.)))
                .truncate()
                .text_color(colors.text_muted)
                .child(bound.scope_label()),
        )
        .child(
            div()
                .flex_none()
                .px(u(tokens.spacing.md))
                .rounded(u(tokens.radius.sm))
                .bg(colors.element)
                .text_color(colors.text_muted)
                .child(bound.role.kind.as_str()),
        )
        .child(link(
            tokens,
            ("sa-bound-role", ix),
            bound.role.name.to_string(),
            cx,
            move |this, cx| this.open(role_target.clone(), cx),
        ))
        .when(!bound.role_found, |row| {
            row.child(
                div()
                    .debug_selector(move || format!("sa-bound-missing-role-{ix}"))
                    .text_color(colors.warning)
                    .child("role not found"),
            )
        })
        .child(div().text_color(colors.text_muted).child("via"))
        .child(link(
            tokens,
            ("sa-bound-binding", ix),
            format!("{} {}", bound.binding.kind.as_str(), bound.binding.name),
            cx,
            move |this, cx| this.open(binding_target.clone(), cx),
        ))
        .children(bound.via_group.as_ref().map(|group| {
            div()
                .debug_selector(move || format!("sa-bound-group-{ix}"))
                .truncate()
                .text_color(colors.text_muted)
                .child(format!("(group {group})"))
        }))
        .into_any_element()
}
