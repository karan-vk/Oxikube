//! Drawing the bulk dialog: the question and counts, the label editor, the typed phrase, the
//! progress and the results.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div, px, uniform_list,
};
use oxikube_domain::command::Propagation;
use oxikube_domain::safety::Risk;
use oxikube_domain::selection::selector_warning;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Disableable as _, Selectable as _, h_flex, v_flex};
use oxikube_ui::spinner::Spinner;
use oxikube_ui::{ActiveTokens as _, Colors, Icon, IconName, Sizable as _, u};
use oxikube_workspace::modal::DIALOG_KEY_CONTEXT;

use super::dialog::{BulkDialog, BulkStage};
use super::kind::BulkKind;
use super::rows::{ROW_HEIGHT, result_row};
use crate::actions::owns_dependents;

/// How many result rows show before the list scrolls.
const VISIBLE_RESULTS: f32 = 10.0;

/// How many lines of counts (one per kind) the question lists before it folds the rest.
const MAX_COUNT_LINES: usize = 6;

impl BulkDialog {
    fn count_noun(&self) -> &'static str {
        match self.kind {
            BulkKind::Restart => "workload",
            _ => "object",
        }
    }

    fn title(&self) -> SharedString {
        let n = self.plan.summary().total();
        let noun = self.count_noun();
        let s = if n == 1 { "" } else { "s" };
        match (self.kind, &self.editor) {
            (BulkKind::Label, editor) => {
                let word = editor.as_ref().map_or("labels", |e| e.field.key());
                format!("Edit {word} of {n} {noun}{s}").into()
            }
            (kind, _) => format!("{} {n} {noun}{s}?", kind.verb()).into(),
        }
    }

    /// One line on what is at stake.
    fn warning(&self) -> Option<String> {
        match self.kind {
            BulkKind::Delete => {
                let names = |kind: &str| {
                    self.plan
                        .targets()
                        .iter()
                        .any(|t| &*t.gvk.kind == kind && t.gvk.group.is_empty())
                };
                if names("Namespace") {
                    Some("Deleting a Namespace deletes everything in it.".into())
                } else if names("PersistentVolume") {
                    Some("Deleting a PersistentVolume can destroy the data on it.".into())
                } else if names("Node") {
                    Some("Deleting a Node removes it from the cluster; its pods are rescheduled.".into())
                } else if self.propagation() == Propagation::Foreground {
                    Some("Dependents are deleted first, then the object.".into())
                } else if self.plan.risk() >= Risk::High {
                    Some("This cannot be undone.".into())
                } else {
                    None
                }
            }
            BulkKind::Restart => Some(
                "Pods are replaced by new ones in rolling updates, as kubectl rollout restart does."
                    .into(),
            ),
            BulkKind::Label => self
                .editor
                .as_ref()
                .and_then(|editor| selector_warning(editor.field))
                .map(str::to_owned),
        }
    }

    /// Whether the dialog asks how dependents are handled: only when an object being deleted can
    /// own some, or the plan already departs from the default.
    fn asks_propagation(&self) -> bool {
        self.kind == BulkKind::Delete
            && (self.propagation() != Propagation::Background
                || self.plan.targets().iter().any(|t| owns_dependents(&t.gvk)))
    }

    fn counts(&self, colors: &Colors) -> AnyElement {
        let lines = self.plan.summary().lines(MAX_COUNT_LINES);
        v_flex()
            .debug_selector(|| "bulk-what".into())
            .gap(u(px(2.)))
            .text_color(colors.text_muted)
            .child(format!("On {}:", self.plan.context()))
            .children(lines.into_iter().map(|line| div().child(line)))
            .into_any_element()
    }

    fn propagation_row(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = self.propagation();
        let tokens = cx.colors();
        let choices = [
            (Propagation::Background, "Background"),
            (Propagation::Foreground, "Foreground"),
            (Propagation::Orphan, "Orphan"),
        ];
        h_flex()
            .debug_selector(|| "bulk-propagation".into())
            .gap(u(px(4.)))
            .items_center()
            .child(div().text_color(tokens.text_muted).child("Dependents"))
            .children(choices.map(|(choice, label)| {
                Button::new(label)
                    .label(label)
                    .small()
                    .selected(choice == current)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_propagation(choice, cx)))
            }))
    }

    fn confirm_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let typed = self.plan.phrase().map(|phrase| {
            v_flex()
                .gap(u(px(4.)))
                .child(
                    div()
                        .debug_selector(|| "bulk-type-prompt".into())
                        .child(format!("Type the cluster name {phrase} to confirm")),
                )
                .child(
                    div()
                        .debug_selector(|| "bulk-type-input".into())
                        .child(Input::new(&self.typed)),
                )
        });
        let editor_problem = self.editor.as_ref().and_then(|editor| editor.problem(cx));
        v_flex()
            .gap(u(px(12.)))
            .child(self.counts(&colors))
            .children(self.warning().map(|warning| {
                h_flex()
                    .debug_selector(|| "bulk-warning".into())
                    .gap(u(px(8.)))
                    .items_start()
                    .text_color(colors.warning)
                    .child(Icon::new(IconName::TriangleAlert).size(u(px(14.))))
                    .child(div().flex_1().min_w_0().child(warning))
            }))
            .children(self.editor.is_some().then(|| self.field_switch(cx)))
            .children(self.editor.is_some().then(|| self.editor_rows(cx)))
            .children(editor_problem.map(|problem| {
                div()
                    .debug_selector(|| "bulk-problem".into())
                    .text_color(colors.error)
                    .child(problem)
            }))
            .children(self.preview(cx))
            .children(self.asks_propagation().then(|| self.propagation_row(cx)))
            .children(typed)
            .children(self.error.clone().map(|error| {
                div()
                    .debug_selector(|| "bulk-error".into())
                    .text_color(colors.error)
                    .child(error)
            }))
            .into_any_element()
    }

    fn running_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let n = self.plan.summary().total();
        let verb = match self.kind {
            BulkKind::Delete => "Deleting",
            BulkKind::Restart => "Restarting",
            BulkKind::Label => "Changing",
        };
        h_flex()
            .debug_selector(|| "bulk-running".into())
            .gap(u(px(8.)))
            .items_center()
            .child(
                Spinner::new()
                    .icon(Icon::new(IconName::LoaderCircle))
                    .color(colors.accent),
            )
            .child(format!(
                "{verb} {n} {}…",
                if n == 1 { "object" } else { "objects" }
            ))
            .into_any_element()
    }

    fn results_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let Some(report) = &self.report else {
            return div().into_any_element();
        };
        let rows = report.rows.len();
        let height = u(px(ROW_HEIGHT * VISIBLE_RESULTS.min(rows as f32)));
        v_flex()
            .gap(u(px(8.)))
            .child(
                div()
                    .debug_selector(|| "bulk-summary".into())
                    .text_color(if report.failed() == 0 {
                        colors.success
                    } else {
                        colors.warning
                    })
                    .child(report.summary()),
            )
            .children(self.error.clone().map(|error| {
                div()
                    .debug_selector(|| "bulk-error".into())
                    .text_color(colors.error)
                    .child(error)
            }))
            .child(
                div()
                    .debug_selector(|| "bulk-results".into())
                    .h(height)
                    .w_full()
                    .child(
                        uniform_list(
                            "bulk-results-list",
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
        let failed = self.report.as_ref().map_or(0, |report| report.failed());
        match self.stage {
            BulkStage::Confirm => {
                let confirm = Button::new("bulk-confirm")
                    .label(self.kind.verb())
                    .disabled(!confirm_enabled)
                    .on_click(cx.listener(|this, _, _, cx| this.confirm(cx)));
                let confirm = if self.kind == BulkKind::Delete {
                    confirm.danger()
                } else {
                    confirm.primary()
                };
                DialogFooter::new()
                    .child(
                        Button::new("bulk-cancel")
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    )
                    .child(
                        div()
                            .debug_selector(|| "bulk-confirm".into())
                            .child(confirm),
                    )
            }
            BulkStage::Running => DialogFooter::new(),
            BulkStage::Done => DialogFooter::new()
                .children((failed > 0).then(|| {
                    div().debug_selector(|| "bulk-retry".into()).child(
                        Button::new("bulk-retry")
                            .label(format!("Retry {failed} failed"))
                            .on_click(cx.listener(|this, _, _, cx| this.retry_failed(cx))),
                    )
                }))
                .child(
                    div().debug_selector(|| "bulk-close".into()).child(
                        Button::new("bulk-close")
                            .label("Close")
                            .primary()
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    ),
                ),
        }
    }
}

impl Render for BulkDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let body = match self.stage {
            BulkStage::Confirm => self.confirm_body(cx),
            BulkStage::Running => self.running_body(cx),
            BulkStage::Done => self.results_body(cx),
        };
        let title: SharedString = if self.stage == BulkStage::Confirm {
            self.title()
        } else {
            match self.kind {
                BulkKind::Delete => "Delete",
                BulkKind::Restart => "Restart",
                BulkKind::Label => "Edit labels",
            }
            .into()
        };
        v_flex()
            .id("bulk-dialog")
            .debug_selector(|| "bulk-dialog".into())
            .key_context(DIALOG_KEY_CONTEXT)
            .track_focus(&self.focus)
            .tab_group()
            .w(u(px(560.)))
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
                    BulkStage::Confirm => this.confirm(cx),
                    BulkStage::Done => this.cancel(cx),
                    BulkStage::Running => {}
                },
            ))
            .child(DialogHeader::new().child(DialogTitle::new().child(title)))
            .child(body)
            .child(self.footer(cx))
    }
}
