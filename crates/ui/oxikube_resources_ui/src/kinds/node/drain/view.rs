//! Drawing the drain dialog: the switches and numbers, the kubectl line, the plan or the
//! per-pod progress, the typed name, and the buttons of each stage.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div, px, uniform_list,
};
use oxikube_domain::drain::DrainPhase;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{Cancel, DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Disableable as _, Selectable as _, h_flex, v_flex};
use oxikube_ui::spinner::Spinner;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};
use oxikube_workspace::modal::DIALOG_KEY_CONTEXT;

use super::dialog::{DrainDialog, DrainLoaded, DrainStage, DrainSwitch};
use super::rows::{ROW_HEIGHT, line_row};

/// How many rows show before the list scrolls.
const VISIBLE_ROWS: f32 = 10.0;

impl DrainDialog {
    fn switch(
        &self,
        id: &'static str,
        label: &'static str,
        flag: DrainSwitch,
        on: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        div().debug_selector(move || id.into()).child(
            Button::new(id)
                .label(label)
                .small()
                .icon(
                    Icon::new(if on { IconName::Check } else { IconName::Minus }).size(u(px(12.))),
                )
                .selected(on)
                .disabled(self.stage != DrainStage::Asking)
                .on_click(cx.listener(move |this, _, _, cx| this.set_flag(flag, !on, cx))),
        )
    }

    fn options(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let asking = self.stage == DrainStage::Asking;
        let number = |id: &'static str, label: &'static str, input| {
            v_flex()
                .gap(u(px(2.)))
                .flex_1()
                .child(
                    div()
                        .text_size(u(px(12.)))
                        .text_color(colors.text_muted)
                        .child(label),
                )
                .child(
                    div()
                        .debug_selector(move || id.into())
                        .child(Input::new(input).small().disabled(!asking)),
                )
        };
        v_flex()
            .gap(u(px(8.)))
            .child(
                h_flex()
                    .gap(u(px(6.)))
                    .flex_wrap()
                    .child(self.switch(
                        "drain-ignore-daemonsets",
                        "Ignore DaemonSets",
                        DrainSwitch::IgnoreDaemonSets,
                        self.flags.ignore_daemonsets,
                        cx,
                    ))
                    .child(self.switch(
                        "drain-delete-emptydir",
                        "Delete emptyDir data",
                        DrainSwitch::DeleteEmptyDirData,
                        self.flags.delete_emptydir_data,
                        cx,
                    ))
                    .child(self.switch(
                        "drain-force",
                        "Force unmanaged pods",
                        DrainSwitch::Force,
                        self.flags.force,
                        cx,
                    )),
            )
            .child(
                h_flex()
                    .gap(u(px(12.)))
                    .child(number("drain-grace", "Grace period (s)", &self.grace))
                    .child(number("drain-timeout", "Timeout (s)", &self.timeout)),
            )
            .child(
                div()
                    .debug_selector(|| "drain-kubectl".into())
                    .px(u(px(8.)))
                    .py(u(px(4.)))
                    .rounded(u(px(4.)))
                    .bg(colors.surface)
                    .text_size(u(px(12.)))
                    .font_family("monospace")
                    .text_color(colors.text_muted)
                    .child(self.kubectl_line()),
            )
            .into_any_element()
    }

    /// The list of lines, virtualised.
    fn list(&self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.lines.len();
        let height = u(px(ROW_HEIGHT * VISIBLE_ROWS.min(rows.max(1) as f32)));
        div()
            .debug_selector(|| "drain-rows".into())
            .h(height)
            .w_full()
            .child(
                uniform_list(
                    "drain-rows-list",
                    rows,
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        let colors = cx.colors();
                        range
                            .map(|ix| line_row(&this.lines, ix, &colors).into_any_element())
                            .collect::<Vec<_>>()
                    }),
                )
                .size_full(),
            )
            .into_any_element()
    }

    /// One line above the list: what the plan does, or how far the drain is.
    fn summary(&self) -> String {
        match self.stage {
            DrainStage::Asking => match &self.plan {
                Some(plan) if !plan.blocked.is_empty() => format!(
                    "{} pod(s) block the drain; nothing will be changed until they are allowed.",
                    plan.blocked.len()
                ),
                Some(plan) if plan.evict.is_empty() => format!(
                    "Cordon the node; no pod to evict ({} left alone).",
                    plan.skipped.len()
                ),
                Some(plan) => format!(
                    "Cordon the node, then evict {} pod(s); {} left alone.",
                    plan.evict.len(),
                    plan.skipped.len()
                ),
                None => String::new(),
            },
            DrainStage::Draining | DrainStage::Done => {
                let counts = self.board.counts();
                let phase = match self.board.phase() {
                    DrainPhase::Starting => "Starting",
                    DrainPhase::Planned => "Cordoning",
                    DrainPhase::Evicting => "Evicting",
                    DrainPhase::Finished(_) => "Finished",
                    DrainPhase::Failed(_) => "Failed",
                    DrainPhase::Cancelled => "Cancelled",
                };
                let mut line = format!(
                    "{phase}: {} of {} pod(s) gone",
                    counts.deleted, counts.total
                );
                if counts.blocked > 0 {
                    line.push_str(&format!(
                        ", {} blocked by a disruption budget",
                        counts.blocked
                    ));
                }
                if counts.failed > 0 {
                    line.push_str(&format!(", {} failed", counts.failed));
                }
                line
            }
        }
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let context = match &self.loaded {
            DrainLoaded::Loading => {
                return h_flex()
                    .debug_selector(|| "drain-loading".into())
                    .gap(u(px(8.)))
                    .items_center()
                    .child(
                        Spinner::new()
                            .icon(Icon::new(IconName::LoaderCircle))
                            .color(colors.accent),
                    )
                    .child("Reading the node's pods…")
                    .into_any_element();
            }
            DrainLoaded::Failed(reason) => {
                return div()
                    .debug_selector(|| "drain-load-error".into())
                    .text_color(colors.error)
                    .child(reason.clone())
                    .into_any_element();
            }
            DrainLoaded::Ready(context) => context,
        };
        let what = format!(
            "Node {} on {}{}",
            self.target.name,
            context.context,
            if context.cordoned {
                " (cordoned already)"
            } else {
                ""
            }
        );
        let asking = self.stage == DrainStage::Asking;
        let typed = asking.then(|| {
            v_flex()
                .gap(u(px(4.)))
                .child(
                    div()
                        .debug_selector(|| "drain-type-prompt".into())
                        .child(format!("Type {} to confirm", self.target.name)),
                )
                .child(
                    div()
                        .debug_selector(|| "drain-type-input".into())
                        .child(Input::new(&self.typed)),
                )
        });
        let blocker = asking
            .then(|| self.start_blocker(cx))
            .flatten()
            .filter(|_| self.plan.as_ref().is_some_and(|p| !p.blocked.is_empty()));
        v_flex()
            .gap(u(px(12.)))
            .child(
                div()
                    .debug_selector(|| "drain-what".into())
                    .text_color(colors.text_muted)
                    .child(what),
            )
            .child(self.options(cx))
            .child(
                div()
                    .debug_selector(|| "drain-summary".into())
                    .child(self.summary()),
            )
            .child(self.list(cx))
            .children(blocker.map(|reason| {
                h_flex()
                    .debug_selector(|| "drain-blocked".into())
                    .gap(u(px(8.)))
                    .items_center()
                    .text_color(colors.error)
                    .child(Icon::new(IconName::TriangleAlert).size(u(px(14.))))
                    .child(div().flex_1().min_w_0().child(reason))
            }))
            .children(typed)
            .children(self.message.clone().map(|message| {
                div()
                    .debug_selector(|| "drain-message".into())
                    .text_color(colors.text_muted)
                    .child(message)
            }))
            .children(self.error.clone().map(|error| {
                div()
                    .debug_selector(|| "drain-error".into())
                    .text_color(colors.error)
                    .child(error)
            }))
            .into_any_element()
    }

    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        match self.stage {
            DrainStage::Asking => DialogFooter::new()
                .child(
                    Button::new("drain-cancel")
                        .label("Cancel")
                        .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                )
                .child(
                    div().debug_selector(|| "drain-start".into()).child(
                        Button::new("drain-start")
                            .label("Drain")
                            .danger()
                            .disabled(!self.can_start(cx))
                            .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
                    ),
                ),
            DrainStage::Draining => DialogFooter::new().child(
                div().debug_selector(|| "drain-stop".into()).child(
                    Button::new("drain-stop")
                        .label("Cancel drain")
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_drain(cx))),
                ),
            ),
            DrainStage::Done => DialogFooter::new()
                .children(self.offers_uncordon().then(|| {
                    div().debug_selector(|| "drain-uncordon".into()).child(
                        Button::new("drain-uncordon")
                            .label("Uncordon")
                            .on_click(cx.listener(|this, _, _, cx| this.uncordon(cx))),
                    )
                }))
                .child(
                    div().debug_selector(|| "drain-close".into()).child(
                        Button::new("drain-close")
                            .label("Close")
                            .primary()
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                    ),
                ),
        }
    }
}

impl Render for DrainDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let title: SharedString = format!("Drain node {}", self.target.name).into();
        v_flex()
            .id("drain-dialog")
            .debug_selector(|| "drain-dialog".into())
            .key_context(DIALOG_KEY_CONTEXT)
            .track_focus(&self.focus)
            .tab_group()
            .w(u(px(600.)))
            .gap(u(tokens.spacing.lg))
            .p(u(tokens.spacing.xxl))
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_lg()
            .on_action(cx.listener(|this, _: &Cancel, _, cx| this.close(cx)))
            .child(DialogHeader::new().child(DialogTitle::new().child(title)))
            .child(self.body(cx))
            .child(self.footer(cx))
    }
}
