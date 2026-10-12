//! Drawing the rollback dialog: the history table, the preview of the chosen revision, the
//! confirmation line.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use oxikube_app::object_label;
use oxikube_domain::command::{self, CommandId};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{Cancel, Confirm, DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::layout::{Disableable as _, StyledExt as _, h_flex, v_flex};
use oxikube_ui::spinner::Spinner;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, u};
use oxikube_workspace::modal::DIALOG_KEY_CONTEXT;

use super::dialog::{RollbackDialog, RollbackStage};
use super::rows::HistoryRow;

impl RollbackDialog {
    fn title(&self) -> SharedString {
        format!("Rollback {}", self.target.name).into()
    }

    /// `Deployment shop/web`: the workload the history belongs to.
    fn subtitle(&self) -> SharedString {
        let workload = self
            .history
            .as_ref()
            .map_or(&self.target, |history| &history.workload);
        object_label(workload).into()
    }

    /// The line about the confirmation: the tier is the command's, read from its declaration.
    pub fn tier_line() -> String {
        let risk = command::lookup(CommandId::WORKLOAD_ROLLBACK)
            .and_then(|meta| meta.risk)
            .map_or("medium", |risk| risk.as_str());
        format!(
            "Risk: {risk}. The pod template is replaced and a rolling update replaces the pods. \
             Asks once; refused on a read-only cluster."
        )
    }

    fn row(&self, row: &HistoryRow, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let revision = row.revision;
        let selected = self.selected == Some(revision);
        let cell = |text: SharedString| div().min_w_0().truncate().child(text);
        h_flex()
            .id(("rollback-row", revision as usize))
            .debug_selector(move || format!("rollback-row-{revision}"))
            .gap(u(tokens.spacing.md))
            .px(u(tokens.spacing.md))
            .py(u(px(3.)))
            .rounded(u(tokens.radius.sm))
            .cursor_pointer()
            .when(selected, |style| style.bg(colors.element_selected))
            .hover(|style| style.bg(colors.element_hover))
            .on_click(cx.listener(move |this, _, _, cx| this.select(revision, cx)))
            .child(
                h_flex()
                    .flex_none()
                    .w(u(px(72.)))
                    .gap(u(px(4.)))
                    .child(div().font_semibold().child(revision.to_string()))
                    .children(row.current.then(|| {
                        div()
                            .debug_selector(move || format!("rollback-current-{revision}"))
                            .text_size(u(px(10.)))
                            .text_color(colors.accent)
                            .child("current")
                    })),
            )
            .child(cell(row.cause.clone()).flex_1())
            .child(
                cell(row.images.clone())
                    .flex_1()
                    .text_color(colors.text_muted),
            )
            .child(
                cell(row.ready.clone())
                    .flex_none()
                    .w(u(px(44.)))
                    .text_color(colors.text_muted),
            )
            .child(
                cell(row.age.clone())
                    .flex_none()
                    .w(u(px(40.)))
                    .text_color(colors.text_muted),
            )
            .into_any_element()
    }

    fn table(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let header = |text: &'static str| div().min_w_0().child(text);
        v_flex()
            .debug_selector(|| "rollback-table".into())
            .gap(u(px(2.)))
            .child(
                h_flex()
                    .gap(u(tokens.spacing.md))
                    .px(u(tokens.spacing.md))
                    .text_size(u(px(11.)))
                    .text_color(colors.text_muted)
                    .child(header("REVISION").flex_none().w(u(px(72.))))
                    .child(header("CHANGE CAUSE").flex_1())
                    .child(header("IMAGES").flex_1())
                    .child(header("READY").flex_none().w(u(px(44.))))
                    .child(header("AGE").flex_none().w(u(px(40.)))),
            )
            .child(
                v_flex()
                    .id("rollback-rows")
                    .max_h(u(px(220.)))
                    .overflow_y_scroll()
                    .gap(u(px(1.)))
                    .children(self.rows.iter().map(|row| self.row(row, cx)))
                    .children((self.hidden > 0).then(|| {
                        div()
                            .debug_selector(|| "rollback-hidden".into())
                            .px(u(tokens.spacing.md))
                            .text_color(colors.text_muted)
                            .child(format!("and {} older revisions", self.hidden))
                    })),
            )
            .into_any_element()
    }

    /// The chosen revision's differences from the running template.
    fn preview(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let Some(row) = self.selected_row() else {
            return div().into_any_element();
        };
        v_flex()
            .debug_selector(|| "rollback-preview".into())
            .gap(u(px(2.)))
            .child(
                div()
                    .text_size(u(px(11.)))
                    .text_color(colors.text_muted)
                    .child(row.preview_title.clone()),
            )
            .children(row.preview.iter().map(|line| {
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(u(px(12.)))
                    .child(line.clone())
            }))
            .into_any_element()
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let notice = |id: &'static str, color, text: String| {
            h_flex()
                .debug_selector(move || id.into())
                .gap(u(px(8.)))
                .items_start()
                .text_size(u(px(12.)))
                .text_color(color)
                .child(Icon::new(IconName::TriangleAlert).size(u(px(14.))))
                .child(div().flex_1().min_w_0().child(text))
                .into_any_element()
        };
        let main = if self.stage == RollbackStage::Loading {
            h_flex()
                .debug_selector(|| "rollback-loading".into())
                .gap(u(px(8.)))
                .items_center()
                .text_color(colors.text_muted)
                .child(
                    Spinner::new()
                        .icon(Icon::new(IconName::LoaderCircle))
                        .color(colors.accent),
                )
                .child("Reading the rollout history…")
                .into_any_element()
        } else if self.history.is_none() {
            // The read failed: the error line below says why; never claim the history is empty.
            div().into_any_element()
        } else if self.rows.is_empty() {
            div()
                .debug_selector(|| "rollback-empty".into())
                .text_color(colors.text_muted)
                .child("No revisions were found: there is nothing to roll back to.")
                .into_any_element()
        } else {
            v_flex()
                .gap(u(px(10.)))
                .child(self.table(cx))
                .child(self.preview(cx))
                .into_any_element()
        };
        v_flex()
            .gap(u(px(12.)))
            .child(
                div()
                    .debug_selector(|| "rollback-subtitle".into())
                    .text_color(colors.text_muted)
                    .child(self.subtitle()),
            )
            .children(self.paused().then(|| {
                notice(
                    "rollback-paused",
                    colors.warning,
                    "This Deployment is paused: kubectl cannot roll back a paused deployment. \
                     Resume it first."
                        .to_owned(),
                )
            }))
            .child(main)
            .child(
                div()
                    .debug_selector(|| "rollback-tier".into())
                    .text_size(u(px(12.)))
                    .text_color(colors.text_muted)
                    .child(self.tier.clone()),
            )
            .children(self.error.clone().map(|error| {
                div()
                    .debug_selector(|| "rollback-error".into())
                    .text_color(colors.error)
                    .child(error)
            }))
            .into_any_element()
    }

    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let running = self.stage == RollbackStage::RollingBack;
        let label: SharedString = if running {
            "Rolling back…".into()
        } else {
            match self.selected {
                Some(revision) if self.selected_row().is_some_and(|row| !row.current) => {
                    format!("Roll back to revision {revision}").into()
                }
                _ => "Roll back".into(),
            }
        };
        DialogFooter::new()
            .child(
                Button::new("rollback-cancel")
                    .label("Cancel")
                    .disabled(running)
                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
            )
            .child(
                div().debug_selector(|| "rollback-confirm".into()).child(
                    Button::new("rollback-confirm")
                        .label(label)
                        .primary()
                        .disabled(!self.can_submit())
                        .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                ),
            )
    }
}

impl Render for RollbackDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        v_flex()
            .id("rollback-dialog")
            .debug_selector(|| "rollback-dialog".into())
            .key_context(DIALOG_KEY_CONTEXT)
            .track_focus(&self.focus)
            .tab_group()
            .w(u(px(720.)))
            .gap(u(tokens.spacing.lg))
            .p(u(tokens.spacing.xxl))
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_lg()
            .on_action(cx.listener(|this, _: &Cancel, _, cx| this.cancel(cx)))
            .on_action(cx.listener(|this, _: &Confirm, _, cx| this.submit(cx)))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                match event.keystroke.key.as_str() {
                    "up" => this.move_selection(-1, cx),
                    "down" => this.move_selection(1, cx),
                    _ => {}
                }
            }))
            .child(DialogHeader::new().child(DialogTitle::new().child(self.title())))
            .child(self.body(cx))
            .child(self.footer(cx))
    }
}
