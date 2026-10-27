//! Drawing the ServiceAccount panel: sections of links above a virtualised bound-roles list.

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, uniform_list,
};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, Tokens, u};

use super::model::{BoundRole, BoundRoles, ServiceAccountFacts};
use super::panel::ServiceAccountPanel;

/// The height of a bound-roles row, at 100 % zoom: uniform, as the virtualised list needs.
const ROW_HEIGHT: f32 = 26.;

/// How many bound roles show before the list scrolls.
const VISIBLE_ROWS: usize = 8;

impl Render for ServiceAccountPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let (Some(facts), Some(cluster)) = (self.facts.clone(), self.cluster(cx)) else {
            return div()
                .id("service-account-panel")
                .debug_selector(|| "service-account-panel".to_owned())
                .p(u(tokens.spacing.lg))
                .text_size(u(tokens.font.small))
                .text_color(tokens.colors.text_muted)
                .child("Reading the ServiceAccount…")
                .into_any_element();
        };
        v_flex()
            .id("service-account-panel")
            .debug_selector(|| "service-account-panel".to_owned())
            .text_color(tokens.colors.text)
            .text_size(u(tokens.font.small))
            .child(self.header(&facts, cx))
            .child(self.secrets(&tokens, &facts, &cluster, cx))
            .child(self.automount(&tokens, &facts))
            .child(self.bound_roles(&tokens, &cluster, cx))
            .into_any_element()
    }
}

impl ServiceAccountPanel {
    /// The account's coordinates, a "Used by" button and the registered header actions.
    fn header(&self, facts: &ServiceAccountFacts, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let target = self.target(cx);
        let extras = super::header_actions(cx);
        h_flex()
            .px(u(tokens.spacing.lg))
            .pt(u(tokens.spacing.lg))
            .pb(u(tokens.spacing.sm))
            .gap(u(tokens.spacing.md))
            .items_center()
            .text_size(u(tokens.font.small))
            .text_color(tokens.colors.text_muted)
            .child(
                div()
                    .debug_selector(|| "sa-identity".to_owned())
                    .child(format!(
                        "system:serviceaccount:{}:{}",
                        facts.namespace, facts.name
                    )),
            )
            .child(div().flex_1())
            .child({
                let target = target.clone();
                div().debug_selector(|| "sa-used-by".to_owned()).child(
                    Button::new("sa-used-by")
                        .xsmall()
                        .ghost()
                        .label("Used by")
                        .tooltip("The workloads whose pods run as this account (u)")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(target) = target.clone() {
                                this.send(Command::ResourceShowUsedBy { target }, cx);
                            }
                        })),
                )
            })
            .children(extras.into_iter().map(|action| {
                let target = target.clone();
                div()
                    .debug_selector(move || format!("sa-action-{}", action.id))
                    .child(
                        Button::new(action.id)
                            .xsmall()
                            .ghost()
                            .label(action.label)
                            .tooltip(action.tooltip)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(target) = &target {
                                    this.send((action.command)(target), cx);
                                }
                            })),
                    )
            }))
            .into_any_element()
    }

    /// Secrets and image pull secrets, each a link to the Secret's detail (never decoded here).
    fn secrets(
        &self,
        tokens: &Tokens,
        facts: &ServiceAccountFacts,
        cluster: &ClusterId,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let secret = |name: &Arc<str>| {
            ResourceRef::namespaced(
                cluster.clone(),
                Gvk::new("", "v1", "Secret"),
                facts.namespace.clone(),
                name.clone(),
            )
        };
        v_flex()
            .child(self.link_section(
                tokens,
                ("sa-secrets", "Secrets"),
                &facts.secrets,
                "None listed. Since Kubernetes 1.24 tokens are requested on demand, not stored as Secrets.",
                &secret,
                cx,
            ))
            .child(self.link_section(
                tokens,
                ("sa-pull-secrets", "Image pull secrets"),
                &facts.image_pull_secrets,
                "None.",
                &secret,
                cx,
            ))
            .into_any_element()
    }

    fn link_section(
        &self,
        tokens: &Tokens,
        (id, title): (&'static str, &'static str),
        names: &[Arc<str>],
        none: &'static str,
        target: &dyn Fn(&Arc<str>) -> ResourceRef,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let links = names.iter().enumerate().map(|(ix, name)| {
            let target = target(name);
            link(tokens, (id, ix), name.to_string(), cx, move |this, cx| {
                this.open(target.clone(), cx);
            })
        });
        v_flex()
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.sm))
            .gap(u(tokens.spacing.sm))
            .child(heading(tokens, id, title, Some(names.len())))
            .child(if names.is_empty() {
                note(tokens, none).into_any_element()
            } else {
                h_flex()
                    .flex_wrap()
                    .gap(u(tokens.spacing.lg))
                    .children(links)
                    .into_any_element()
            })
            .into_any_element()
    }

    /// The automount flag and what it means for a Pod, including the default when unset.
    fn automount(&self, tokens: &Tokens, facts: &ServiceAccountFacts) -> AnyElement {
        v_flex()
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.sm))
            .gap(u(tokens.spacing.sm))
            .child(heading(tokens, "sa-automount", "Automount token", None))
            .child(
                h_flex()
                    .gap(u(tokens.spacing.md))
                    .items_center()
                    .child(
                        div()
                            .debug_selector(|| "sa-automount-value".to_owned())
                            .px(u(tokens.spacing.md))
                            .rounded(u(tokens.radius.sm))
                            .bg(tokens.colors.element)
                            .child(facts.automount.label()),
                    )
                    .child(
                        div()
                            .debug_selector(|| "sa-automount-note".to_owned())
                            .text_size(u(tokens.font.small))
                            .text_color(tokens.colors.text_muted)
                            .child(facts.automount.explanation()),
                    ),
            )
            .into_any_element()
    }

    /// The roles bound to the account: a loading row, a note when nothing is bound, a note on
    /// the kinds the user may not list, and the rows.
    fn bound_roles(
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

fn heading(tokens: &Tokens, id: &'static str, title: &str, count: Option<usize>) -> AnyElement {
    div()
        .debug_selector(move || format!("{id}-heading"))
        .font_semibold()
        .text_size(u(tokens.font.small))
        .child(match count {
            Some(count) => format!("{title} ({count})"),
            None => title.to_owned(),
        })
        .into_any_element()
}

fn note(tokens: &Tokens, text: &'static str) -> gpui::Div {
    div()
        .text_size(u(tokens.font.small))
        .text_color(tokens.colors.text_muted)
        .child(text)
}

/// A clickable name in the accent colour.
fn link(
    tokens: &Tokens,
    id: (&'static str, usize),
    text: String,
    cx: &mut Context<ServiceAccountPanel>,
    open: impl Fn(&mut ServiceAccountPanel, &mut Context<ServiceAccountPanel>) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .debug_selector(move || format!("{}-{}", id.0, id.1))
        .cursor_pointer()
        .truncate()
        .text_color(tokens.colors.accent)
        .hover(|style| style.underline())
        .child(text)
        .on_click(cx.listener(move |this, _, _, cx| open(this, cx)))
        .into_any_element()
}
