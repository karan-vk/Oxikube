//! Drawing the page: the toolbar (filter, counts), the table and, under it, the detail.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div, px,
};
use oxikube_keymap::{KeyContextBuilder, contexts};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::table::Table;
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::actions::{
    CloseDetail, FocusFilter, FocusTable, NextTab, PreviousTab, RevealValues, SelectFirst,
    SelectLast, SelectNext, SelectPrevious, ToggleValuesMode,
};
use super::model::Phase;
use super::view::HelmReleasesView;

impl HelmReleasesView {
    fn key_context(&self) -> gpui::KeyContext {
        let mut builder = KeyContextBuilder::new(contexts::HELM_RELEASES);
        builder.flag_if(self.editing, contexts::EDITING);
        builder.build()
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let (shown, total, problem, phase) = self.table.read(cx, |d| {
            (
                d.len(),
                d.total(),
                d.filter().problem().map(ToString::to_string),
                d.phase().clone(),
            )
        });
        let count: SharedString = match (&phase, shown == total) {
            (Phase::Loading, _) if total == 0 => "Reading releases...".into(),
            (_, true) => format!("{total} release{}", if total == 1 { "" } else { "s" }).into(),
            _ => format!("{shown} of {total} releases").into(),
        };
        h_flex()
            .id("helm-releases-toolbar")
            .debug_selector(|| "helm-releases-toolbar".into())
            .flex_none()
            .gap(u(tokens.spacing.md))
            .px(u(tokens.spacing.md))
            .py(u(tokens.spacing.sm))
            .items_center()
            .border_b_1()
            .border_color(colors.border_variant)
            .child(
                div()
                    .debug_selector(|| "helm-releases-filter".into())
                    .flex_none()
                    .w(u(px(240.)))
                    .child(Input::new(&self.filter).xsmall()),
            )
            .child(
                div()
                    .debug_selector(|| "helm-releases-count".into())
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .child(count),
            )
            .children(problem.map(|why| {
                div()
                    .debug_selector(|| "helm-releases-filter-problem".into())
                    .text_size(u(tokens.font.small))
                    .text_color(colors.warning)
                    .child(SharedString::from(why))
            }))
            .child(div().flex_1())
            .children(matches!(phase, Phase::Failed(_)).then(|| {
                div().debug_selector(|| "helm-releases-retry".into()).child(
                    Button::new("helm-releases-retry-button")
                        .label("Retry")
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                )
            }))
            .into_any_element()
    }
}

impl Render for HelmReleasesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let toolbar = self.toolbar(cx);
        let detail = self.detail_pane(cx);
        v_flex()
            .id("helm-releases")
            .debug_selector(|| "helm-releases".into())
            .key_context(self.key_context())
            .track_focus(&self.focus)
            .size_full()
            .bg(tokens.colors.background)
            .text_color(tokens.colors.text)
            .text_size(u(tokens.font.body))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &SelectFirst, _, cx| this.select_end(false, cx)))
            .on_action(cx.listener(|this, _: &SelectLast, _, cx| this.select_end(true, cx)))
            .on_action(
                cx.listener(|this, _: &FocusFilter, window, cx| this.focus_filter(window, cx)),
            )
            .on_action(cx.listener(|this, _: &FocusTable, window, cx| this.focus_table(window, cx)))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| this.step_tab(true, cx)))
            .on_action(cx.listener(|this, _: &PreviousTab, _, cx| this.step_tab(false, cx)))
            .on_action(cx.listener(|this, _: &ToggleValuesMode, _, cx| this.toggle_values_mode(cx)))
            .on_action(cx.listener(|this, _: &RevealValues, _, cx| this.toggle_reveal(cx)))
            .on_action(cx.listener(|this, _: &CloseDetail, _, cx| this.close_detail(cx)))
            .child(toolbar)
            .child(
                div()
                    .id("helm-releases-table")
                    .debug_selector(|| "helm-releases-table".into())
                    .flex_1()
                    .min_h(u(px(120.)))
                    .w_full()
                    .child(Table::new(&self.table).bordered(false)),
            )
            .children(detail.map(|pane| {
                div()
                    .id("helm-release-detail-slot")
                    .flex_1()
                    .min_h(u(px(180.)))
                    .w_full()
                    .border_t_1()
                    .border_color(tokens.colors.border)
                    .child(pane)
            }))
    }
}

impl HelmReleasesView {
    /// Shows the next (or previous) tab of the open detail.
    pub fn step_tab(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.detail.as_ref().map(|d| d.tab) else {
            return;
        };
        self.set_tab(if forward { tab.next() } else { tab.previous() }, cx);
    }
}
