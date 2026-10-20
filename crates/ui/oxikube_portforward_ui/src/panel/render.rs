//! Drawing the panel: the toolbar (filter, cluster, row actions), the table and the status line.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div, px,
};
use oxikube_keymap::{KeyContextBuilder, contexts};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::table::Table;
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::PortForwardPanel;
use super::actions::{
    Close, CopyUrlSelected, FocusFilter, FocusTable, OpenSelected, RemoveSelected, RestartSelected,
    SelectFirst, SelectLast, SelectNext, SelectPrevious, StopSelected,
};

impl PortForwardPanel {
    fn key_context(&self) -> gpui::KeyContext {
        let mut builder = KeyContextBuilder::new(contexts::PORT_FORWARDS);
        builder.flag_if(self.editing, contexts::EDITING);
        builder.build()
    }

    /// A button of the toolbar.
    fn button(
        id: &'static str,
        label: &'static str,
        enabled: bool,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(id)
            .flex_none()
            .debug_selector(move || id.to_owned())
            .child(
                Button::new(id)
                    .label(label)
                    .ghost()
                    .xsmall()
                    .disabled(!enabled)
                    .on_click(cx.listener(move |panel, _, window, cx| on_click(panel, window, cx))),
            )
            .into_any_element()
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let model = self.model(cx);
        let selected = model.selected();
        let has_url = selected.is_some_and(|row| row.url.is_some());
        let running = selected.is_some_and(|row| row.running);
        let has_selection = selected.is_some();
        let cluster_label: SharedString = match model.cluster_filter() {
            None => "All clusters".into(),
            Some(id) => model
                .clusters()
                .into_iter()
                .find(|(cluster, _)| cluster == id)
                .map_or_else(|| id.to_string().into(), |(_, label)| label),
        };
        let several_clusters = model.clusters().len() > 1 || model.cluster_filter().is_some();
        h_flex()
            .id("port-forwards-toolbar")
            .debug_selector(|| "port-forwards-toolbar".into())
            .flex_none()
            .flex_wrap()
            .gap(u(tokens.spacing.sm))
            .px(u(tokens.spacing.md))
            .py(u(tokens.spacing.sm))
            .items_center()
            .border_b_1()
            .border_color(tokens.colors.border_variant)
            .child(
                div()
                    .debug_selector(|| "port-forwards-filter".into())
                    .flex_none()
                    .w(u(px(220.)))
                    .child(Input::new(&self.filter).xsmall()),
            )
            .children(several_clusters.then(|| {
                div()
                    .debug_selector(|| "port-forwards-cluster".into())
                    .child(
                        Button::new("port-forwards-cluster")
                            .label(cluster_label)
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(|panel, _, _, cx| panel.cycle_cluster(cx))),
                    )
            }))
            .child(div().flex_1())
            .child(if running || !has_selection {
                Self::button(
                    "port-forwards-stop",
                    "Stop",
                    running,
                    |panel, _, cx| panel.stop_selected(cx),
                    cx,
                )
            } else {
                Self::button(
                    "port-forwards-start",
                    "Start",
                    true,
                    |panel, _, cx| panel.restart_selected(cx),
                    cx,
                )
            })
            .child(Self::button(
                "port-forwards-restart",
                "Restart",
                has_selection,
                |panel, _, cx| panel.restart_selected(cx),
                cx,
            ))
            .child(Self::button(
                "port-forwards-remove",
                "Remove",
                has_selection,
                |panel, _, cx| panel.remove_selected(cx),
                cx,
            ))
            .child(Self::button(
                "port-forwards-copy",
                "Copy URL",
                has_url,
                |panel, _, cx| panel.copy_url_selected(cx),
                cx,
            ))
            .child(Self::button(
                "port-forwards-open",
                "Open in Browser",
                has_url,
                |panel, _, cx| panel.open_selected(cx),
                cx,
            ))
            .into_any_element()
    }

    /// The line under the table: why the selected forward is not fine, else the counts.
    fn status_line(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let model = self.model(cx);
        let detail = model.selected().and_then(|row| row.detail.clone());
        let (text, colour) = match detail {
            Some(detail) => (detail, tokens.colors.error),
            None => {
                let (shown, total, failed) = (model.len(), model.total(), model.failed());
                let mut text = if shown == total {
                    format!("{total} port-forward{}", if total == 1 { "" } else { "s" })
                } else {
                    format!("{shown} of {total} port-forwards")
                };
                if failed > 0 {
                    text.push_str(&format!(" \u{b7} {failed} failed"));
                }
                (text.into(), tokens.colors.text_muted)
            }
        };
        div()
            .debug_selector(|| "port-forwards-status".into())
            .flex_none()
            .px(u(tokens.spacing.md))
            .py(u(px(3.)))
            .border_t_1()
            .border_color(tokens.colors.border_variant)
            .text_size(u(tokens.font.small))
            .text_color(colour)
            .child(text)
            .into_any_element()
    }
}

impl Render for PortForwardPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let toolbar = self.toolbar(cx);
        let status = self.status_line(cx);
        v_flex()
            .id("port-forwards-panel")
            .debug_selector(|| "port-forwards-panel".into())
            .key_context(self.key_context())
            .track_focus(&self.focus)
            .size_full()
            .bg(tokens.colors.background)
            .text_color(tokens.colors.text)
            .text_size(u(tokens.font.body))
            .on_action(cx.listener(|panel, _: &SelectNext, _, cx| panel.step(1, cx)))
            .on_action(cx.listener(|panel, _: &SelectPrevious, _, cx| panel.step(-1, cx)))
            .on_action(cx.listener(|panel, _: &SelectFirst, _, cx| panel.select_end(false, cx)))
            .on_action(cx.listener(|panel, _: &SelectLast, _, cx| panel.select_end(true, cx)))
            .on_action(cx.listener(|panel, _: &StopSelected, _, cx| panel.stop_selected(cx)))
            .on_action(cx.listener(|panel, _: &RestartSelected, _, cx| panel.restart_selected(cx)))
            .on_action(cx.listener(|panel, _: &RemoveSelected, _, cx| panel.remove_selected(cx)))
            .on_action(cx.listener(|panel, _: &CopyUrlSelected, _, cx| panel.copy_url_selected(cx)))
            .on_action(cx.listener(|panel, _: &OpenSelected, _, cx| panel.open_selected(cx)))
            .on_action(cx.listener(|panel, _: &FocusFilter, window, cx| {
                panel.focus_filter(window, cx);
            }))
            .on_action(cx.listener(|panel, _: &FocusTable, window, cx| {
                panel.focus_table(window, cx);
            }))
            .on_action(cx.listener(|panel, _: &Close, _, cx| panel.close(cx)))
            .child(toolbar)
            .child(
                div()
                    .id("port-forwards-table")
                    .debug_selector(|| "port-forwards-table".into())
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(Table::new(&self.table).bordered(false)),
            )
            .child(status)
    }
}
