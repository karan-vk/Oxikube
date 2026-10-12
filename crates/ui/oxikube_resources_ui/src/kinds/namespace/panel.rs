//! The Namespace panel above the detail's Overview: the phase, what holds a terminating
//! namespace (finalizers and the controller's blockers), HNC's markers, "Use namespace", and the
//! allocation section (E13-S11): the namespace's requests, limits and live usage against its
//! quotas.

use gpui::{
    AnyView, App, AppContext as _, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Subscription, Task, WeakEntity, Window, div, px,
};
use oxikube_domain::view::{NamespacePhase, NamespaceSummary};
use oxikube_settings::{MetricsSettings, Settings as _};
use oxikube_ui::button::Button;
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::allocation::{AllocState, TICK, TICK_WAITING, quotas_block};
use crate::detail::DetailView;
use crate::kinds::allocation::{allocation_section, muted, poll_usage};
use crate::table::ToneColors;

/// Builds the panel for one detail view (a [`DetailRenderer`](crate::kinds::DetailRenderer)
/// factory).
pub(super) fn build(detail: WeakEntity<DetailView>, _: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| NamespacePanel::new(detail, cx)).into()
}

/// The panel.
pub struct NamespacePanel {
    pub(super) detail: WeakEntity<DetailView>,
    /// The allocation section: its inputs, sums and notes.
    pub(super) alloc: AllocState,
    _follow: Option<Subscription>,
    /// Looks at the namespace's pods and quotas in the store.
    _tick: Task<()>,
    /// Polls the metrics port while the panel is open (dropped with it).
    _usage: Task<()>,
    /// Sums again when `metrics.gpu_vendors` changes.
    _gpus: Subscription,
}

impl NamespacePanel {
    fn new(detail: WeakEntity<DetailView>, cx: &mut Context<Self>) -> Self {
        // Follow the object: a namespace that starts terminating redraws the panel.
        let follow = detail
            .upgrade()
            .map(|view| cx.observe(&view, |_, _, cx| cx.notify()));
        let tick = cx.spawn(async move |this, cx| {
            let mut wait = TICK_WAITING;
            loop {
                cx.background_executor().timer(wait).await;
                let next = this.update(cx, |panel, cx| {
                    if panel.refresh_allocation(cx) {
                        cx.notify();
                    }
                    if panel.alloc.view().is_some() {
                        TICK
                    } else {
                        TICK_WAITING
                    }
                });
                match next {
                    Ok(next) => wait = next,
                    Err(_) => break,
                }
            }
        });
        let usage = poll_usage(
            cx,
            |panel: &Self, cx| panel.usage_target(cx),
            |panel, state, cx| panel.usage_arrived(state, cx),
        );
        let gpus = MetricsSettings::observe_in(cx, |panel: &mut Self, cx| {
            panel.alloc.invalidate();
            panel.refresh_allocation(cx);
            cx.notify();
        });
        // The detail is leased while it builds its panel: the first look is the next thing the
        // foreground does.
        cx.spawn(async move |this, cx| {
            this.update(cx, |panel, cx| {
                if panel.refresh_allocation(cx) {
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        Self {
            detail,
            alloc: AllocState::default(),
            _follow: follow,
            _tick: tick,
            _usage: usage,
            _gpus: gpus,
        }
    }

    /// The namespace the detail shows, read off the object (one small JSON walk per draw).
    fn summary(&self, cx: &App) -> Option<NamespaceSummary> {
        let detail = self.detail.upgrade()?;
        let detail = detail.read(cx);
        NamespaceSummary::from_resource(detail.resource()?).ok()
    }

    /// Sends `namespace::Select` with this one namespace: "use this namespace" (k9s `u`).
    fn use_namespace(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let (target, dispatcher) = {
            let detail = detail.read(cx);
            (detail.target().clone(), detail.dispatcher())
        };
        dispatcher.dispatch(super::actions::use_command(&target), cx);
    }
}

impl Render for NamespacePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let tones = ToneColors::current(cx);
        let Some(ns) = self.summary(cx) else {
            return div().into_any_element();
        };
        let phase_colour = match ns.phase {
            NamespacePhase::Active => colors.success,
            NamespacePhase::Terminating => colors.warning,
            NamespacePhase::Unknown => colors.text_muted,
        };
        let line = |id: &str, text: String| {
            let id = id.to_owned();
            div()
                .debug_selector(move || id)
                .w_full()
                .text_size(u(tokens.font.small))
                .text_color(colors.text_muted)
                .child(text)
        };
        let mut lines = Vec::new();
        if ns.is_terminating() {
            let mut text = String::from("Terminating");
            if let Some(since) = ns.deletion {
                text.push_str(&format!(" since {since}"));
            }
            text.push_str(": its contents are being removed.");
            lines.push(line("namespace-terminating", text).text_color(colors.warning));
            let finalizers: Vec<&str> = ns.all_finalizers().map(|f| f.as_ref()).collect();
            if !finalizers.is_empty() {
                lines.push(line(
                    "namespace-finalizers",
                    format!("Waiting for finalizers: {}", finalizers.join(", ")),
                ));
            }
            for (ix, blocker) in ns.blockers().enumerate() {
                let reason = blocker.message.as_deref().or(blocker.reason.as_deref());
                let text = match reason {
                    Some(reason) => format!("{}: {reason}", blocker.kind),
                    None => blocker.kind.to_string(),
                };
                lines.push(line(&format!("namespace-blocker-{ix}"), text));
            }
        }
        if let Some(parent) = &ns.hnc.subnamespace_of {
            lines.push(line(
                "namespace-hnc",
                format!("HNC subnamespace of {parent}"),
            ));
        } else if ns.hnc.managed {
            lines.push(line("namespace-hnc", "Managed by HNC".to_owned()));
        }
        if let Some(reason) = ns.protected() {
            lines.push(line("namespace-protected", reason.to_owned()).text_color(colors.warning));
        }
        v_flex()
            .debug_selector(|| "namespace-panel".to_owned())
            .w_full()
            .gap(u(px(4.)))
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.md))
            .border_b_1()
            .border_color(colors.border_variant)
            .child(
                h_flex()
                    .items_center()
                    .gap(u(tokens.spacing.md))
                    .child(
                        div()
                            .debug_selector(|| "namespace-phase".to_owned())
                            .text_color(phase_colour)
                            .child(ns.phase.as_str()),
                    )
                    .child(div().flex_1())
                    .child(
                        div().debug_selector(|| "namespace-use".to_owned()).child(
                            Button::new("namespace-use")
                                .label("Use namespace")
                                .xsmall()
                                .tooltip("Watch only this namespace (u)")
                                .on_click(cx.listener(|this, _, _, cx| this.use_namespace(cx))),
                        ),
                    ),
            )
            .children(lines)
            .child(self.allocation(&tokens, &tones))
            .into_any_element()
    }
}

impl NamespacePanel {
    /// The allocation section: the aggregate against the quotas, then the quotas themselves.
    fn allocation(&self, tokens: &oxikube_ui::Tokens, tones: &ToneColors) -> gpui::AnyElement {
        let muted = |text: String, id: &'static str| {
            muted(tokens, text).debug_selector(move || id.to_owned())
        };
        let heading = |text: &str, id: &'static str| {
            div()
                .debug_selector(move || id.to_owned())
                .pt(u(tokens.spacing.lg))
                .pb(u(tokens.spacing.sm))
                .text_size(u(tokens.font.small))
                .font_semibold()
                .text_color(tokens.colors.text_muted)
                .child(text.to_uppercase())
        };
        let mut out = v_flex()
            .w_full()
            .child(heading("Allocation", "namespace-allocation-heading"));
        match self.alloc.view() {
            Some(view) => {
                if self.alloc.quotas().is_empty() && self.alloc.quota_note().is_none() {
                    out = out.child(muted(
                        "No ResourceQuota in this namespace: totals only.".to_owned(),
                        "namespace-allocation-versus",
                    ));
                }
                out = out.child(allocation_section(
                    tokens,
                    tones,
                    view,
                    "namespace-allocation",
                ));
            }
            None => {
                out = out.child(muted(
                    self.alloc
                        .note()
                        .unwrap_or("Reading the pods of this namespace...")
                        .to_owned(),
                    "namespace-allocation-pending",
                ));
            }
        }
        if let Some(note) = self.alloc.quota_note() {
            out = out.child(muted(note.to_owned(), "namespace-quota-note"));
        }
        if !self.alloc.quotas().is_empty() {
            out = out
                .child(heading("Quotas", "namespace-quotas-heading"))
                .child(quotas_block(tokens, self.alloc.quotas()));
        }
        out.into_any_element()
    }
}

/// The panel as a typed entity, for tests.
#[cfg(test)]
pub(crate) fn of(view: &AnyView) -> gpui::Entity<NamespacePanel> {
    view.clone()
        .downcast::<NamespacePanel>()
        .expect("a namespace panel")
}

/// What the tests look at: the panel's state, not its pixels.
#[cfg(test)]
impl NamespacePanel {
    /// The allocation section as last built.
    pub(crate) fn allocation_view(&self) -> Option<&oxikube_app::allocation::AllocationView> {
        self.alloc.view()
    }

    /// Why the allocation section is missing, if it is.
    pub(crate) fn allocation_note(&self) -> Option<&str> {
        self.alloc.note()
    }

    /// The quota lines as drawn: per quota, its name and `(resource, used / hard (n%))` lines.
    pub(crate) fn quota_lines(&self) -> Vec<(String, Vec<(String, String)>)> {
        self.alloc
            .quotas()
            .iter()
            .map(|quota| {
                let lines = quota
                    .lines
                    .iter()
                    .map(|l| (l.resource.to_string(), super::allocation::line_text(l)))
                    .collect();
                (quota.name.to_string(), lines)
            })
            .collect()
    }

    /// Why the quotas are missing, if they are.
    pub(crate) fn quota_note(&self) -> Option<&str> {
        self.alloc.quota_note()
    }
}
