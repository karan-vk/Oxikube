//! Drawing the ServiceAccount panel: sections of links above a virtualised bound-roles list.

use std::sync::Arc;

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div,
};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, Tokens, u};

use super::model::ServiceAccountFacts;
use super::panel::ServiceAccountPanel;

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
}

pub(super) fn heading(
    tokens: &Tokens,
    id: &'static str,
    title: &str,
    count: Option<usize>,
) -> AnyElement {
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

pub(super) fn note(tokens: &Tokens, text: &'static str) -> gpui::Div {
    div()
        .text_size(u(tokens.font.small))
        .text_color(tokens.colors.text_muted)
        .child(text)
}

/// A clickable name in the accent colour.
pub(super) fn link(
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
