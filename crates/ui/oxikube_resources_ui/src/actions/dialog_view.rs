//! Drawing the delete dialog: the question, the propagation choice, the typed name, the progress
//! and the results.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div, px, uniform_list,
};
use oxikube_app::object_label;
use oxikube_domain::command::Propagation;
use oxikube_domain::safety::Risk;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Disableable as _, Selectable as _, h_flex, v_flex};
use oxikube_ui::spinner::Spinner;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};
use oxikube_workspace::modal::DIALOG_KEY_CONTEXT;

use super::dependents::owns_dependents;
use super::dialog::{DeleteDialog, Stage};
use super::results::{ROW_HEIGHT, result_row};

/// How many result rows show before the list scrolls.
const VISIBLE_RESULTS: f32 = 10.0;

impl DeleteDialog {
    fn title(&self) -> SharedString {
        match self.plan.items() {
            [only] => format!("Delete {}?", object_label(&only.target)).into(),
            items => format!("Delete {} objects?", items.len()).into(),
        }
    }

    /// "3 Pods, 1 ConfigMap on kind-oxikube".
    fn what(&self) -> String {
        let kinds: Vec<String> = self
            .plan
            .kinds()
            .into_iter()
            .map(|(kind, n)| format!("{n} {kind}{}", if n == 1 { "" } else { "s" }))
            .collect();
        format!("{} on {}", kinds.join(", "), self.plan.context())
    }

    /// One line on what is at stake, for the objects whose delete reaches further than the object.
    fn warning(&self) -> Option<&'static str> {
        let names = |kind: &str| {
            self.plan
                .items()
                .iter()
                .any(|item| &*item.target.gvk.kind == kind && item.target.gvk.group.is_empty())
        };
        if names("Namespace") {
            Some("Deleting a Namespace deletes everything in it.")
        } else if names("PersistentVolume") {
            Some("Deleting a PersistentVolume can destroy the data on it.")
        } else if names("Node") {
            Some("Deleting a Node removes it from the cluster; its pods are rescheduled.")
        } else if self.plan.propagation() == Propagation::Foreground {
            Some("Dependents are deleted first, then the object.")
        } else if self.plan.risk() >= Risk::High {
            Some("This cannot be undone.")
        } else {
            None
        }
    }

    /// Whether the dialog asks how dependents are handled: only when an object being deleted can
    /// own some, or the plan already departs from the default. Other kinds delete with
    /// Background, silently.
    fn asks_propagation(&self) -> bool {
        self.plan.propagation() != Propagation::Background
            || self
                .plan
                .items()
                .iter()
                .any(|item| owns_dependents(&item.target.gvk))
    }

    fn propagation_row(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = self.plan.propagation();
        let tokens = cx.colors();
        let choices = [
            (Propagation::Background, "Background"),
            (Propagation::Foreground, "Foreground"),
            (Propagation::Orphan, "Orphan"),
        ];
        let note = match current {
            Propagation::Background => "Delete now; the garbage collector removes dependents.",
            Propagation::Foreground => "Delete dependents first, then the object.",
            Propagation::Orphan => "Leave dependents in place.",
        };
        v_flex()
            .gap(u(px(4.)))
            .child(
                h_flex()
                    .gap(u(px(4.)))
                    .items_center()
                    .child(div().text_color(tokens.text_muted).child("Dependents"))
                    .children(choices.map(|(choice, label)| {
                        Button::new(label)
                            .label(label)
                            .small()
                            .selected(choice == current)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.set_propagation(choice, cx)),
                            )
                    })),
            )
            .child(
                div()
                    .debug_selector(|| "delete-propagation-note".into())
                    .text_size(u(px(12.)))
                    .text_color(tokens.text_muted)
                    .child(note),
            )
    }

    fn confirm_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let typed = self.plan.phrase().map(|phrase| {
            let ask = if self.plan.items().len() == 1 {
                format!("Type {phrase} to confirm")
            } else {
                format!("Type the cluster name {phrase} to confirm")
            };
            v_flex()
                .gap(u(px(4.)))
                .child(
                    div()
                        .debug_selector(|| "delete-type-prompt".into())
                        .child(ask),
                )
                .child(
                    div()
                        .debug_selector(|| "delete-type-input".into())
                        .child(Input::new(&self.typed)),
                )
        });
        v_flex()
            .gap(u(px(12.)))
            .child(
                div()
                    .debug_selector(|| "delete-what".into())
                    .text_color(colors.text_muted)
                    .child(self.what()),
            )
            .children(self.warning().map(|warning| {
                h_flex()
                    .debug_selector(|| "delete-warning".into())
                    .gap(u(px(8.)))
                    .items_center()
                    .text_color(colors.warning)
                    .child(Icon::new(IconName::TriangleAlert).size(u(px(14.))))
                    .child(warning)
            }))
            .children(self.asks_propagation().then(|| self.propagation_row(cx)))
            .children(typed)
            .children(self.error.clone().map(|error| {
                div()
                    .debug_selector(|| "delete-error".into())
                    .text_color(colors.error)
                    .child(error)
            }))
            .into_any_element()
    }

    fn running_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let n = self.plan.items().len();
        h_flex()
            .debug_selector(|| "delete-running".into())
            .gap(u(px(8.)))
            .items_center()
            .child(
                Spinner::new()
                    .icon(Icon::new(IconName::LoaderCircle))
                    .color(colors.accent),
            )
            .child(format!(
                "Deleting {n} {}…",
                if n == 1 { "object" } else { "objects" }
            ))
            .into_any_element()
    }

    fn results_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let Some(report) = &self.report else {
            return div().into_any_element();
        };
        let rows = report.items.len();
        let height = u(px(ROW_HEIGHT * VISIBLE_RESULTS.min(rows as f32)));
        v_flex()
            .gap(u(px(8.)))
            .child(
                div()
                    .debug_selector(|| "delete-summary".into())
                    .text_color(if report.failed() == 0 {
                        colors.success
                    } else {
                        colors.warning
                    })
                    .child(report.summary()),
            )
            .child(
                div()
                    .debug_selector(|| "delete-results".into())
                    .h(height)
                    .w_full()
                    .child(
                        uniform_list(
                            "delete-results-list",
                            rows,
                            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                let colors = cx.colors();
                                let Some(report) = &this.report else {
                                    return Vec::new();
                                };
                                range
                                    .map(|ix| result_row(report, ix, &colors).into_any_element())
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .size_full(),
                    ),
            )
            .into_any_element()
    }

    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let confirm_enabled = self.can_confirm(cx);
        match self.stage {
            Stage::Confirm => DialogFooter::new()
                .child(
                    Button::new("delete-cancel")
                        .label("Cancel")
                        .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                )
                .child(
                    div().debug_selector(|| "delete-confirm".into()).child(
                        Button::new("delete-confirm")
                            .label("Delete")
                            .danger()
                            .disabled(!confirm_enabled)
                            .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))),
                    ),
                ),
            Stage::Running => DialogFooter::new(),
            Stage::Done => DialogFooter::new().child(
                div().debug_selector(|| "delete-close".into()).child(
                    Button::new("delete-close")
                        .label("Close")
                        .primary()
                        .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                ),
            ),
        }
    }
}

impl Render for DeleteDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let body = match self.stage {
            Stage::Confirm => self.confirm_body(cx),
            Stage::Running => self.running_body(cx),
            Stage::Done => self.results_body(cx),
        };
        let title = if self.stage == Stage::Confirm {
            self.title()
        } else {
            "Delete".into()
        };
        v_flex()
            .id("delete-dialog")
            .debug_selector(|| "delete-dialog".into())
            .key_context(DIALOG_KEY_CONTEXT)
            .track_focus(&self.focus)
            .tab_group()
            .w(u(px(520.)))
            .gap(u(tokens.spacing.lg))
            .p(u(tokens.spacing.xxl))
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_lg()
            .on_action(cx.listener(|this, _: &oxikube_ui::dialog::Cancel, _, cx| this.cancel(cx)))
            .on_action(cx.listener(
                |this, _: &oxikube_ui::dialog::Confirm, _, cx| match this.stage {
                    Stage::Confirm => this.confirm(cx),
                    Stage::Done => this.cancel(cx),
                    Stage::Running => {}
                },
            ))
            .child(DialogHeader::new().child(DialogTitle::new().child(title)))
            .child(body)
            .child(self.footer(cx))
    }
}
