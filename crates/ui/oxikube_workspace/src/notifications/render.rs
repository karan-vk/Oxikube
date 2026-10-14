//! Drawing the [`NotificationPanel`]: the header and the virtualised rows. Rows come ready-made
//! from the centre (no formatting here), and only the visible ones are built.

use gpui::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px, uniform_list,
};
use oxikube_ports::NotificationLevel;
use oxikube_ui::{
    ActiveTokens as _, Colors, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    layout::{Disableable as _, h_flex, v_flex},
    u,
};

use super::centre::NotificationRow;
use super::panel::NotificationPanel;
use super::{
    Close, MarkAllRead, MarkSelectedRead, NOTIFICATION_PANEL_CONTEXT, OpenSelected, SelectNext,
    SelectPrevious,
};

/// Row height in unscaled pixels: the list is uniform.
pub const ROW_HEIGHT: f32 = 76.;

/// What the empty centre says.
const EMPTY: &str = "No notifications. Warning events of watched kinds (pods, workloads, nodes, \
     claims) in your open clusters show up here.";

/// The icon and colour of a level.
pub(super) fn level_style(level: NotificationLevel, colors: &Colors) -> (IconName, Hsla) {
    match level {
        NotificationLevel::Info => (IconName::Info, colors.info),
        NotificationLevel::Success => (IconName::CircleCheck, colors.success),
        NotificationLevel::Warning => (IconName::TriangleAlert, colors.warning),
        NotificationLevel::Error => (IconName::CircleX, colors.error),
    }
}

impl Render for NotificationPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let count = self.rows(cx).len();
        let body = if count == 0 {
            div()
                .id("notification-empty")
                .debug_selector(|| "notification-empty".to_owned())
                .p(u(tokens.spacing.lg))
                .text_color(tokens.colors.text_muted)
                .child(EMPTY)
                .into_any_element()
        } else {
            uniform_list(
                "notification-rows",
                count,
                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                    let rows = this.rows(cx);
                    range
                        .filter_map(|ix| rows.get(ix).map(|row| this.row(ix, row, cx)))
                        .collect::<Vec<AnyElement>>()
                }),
            )
            .track_scroll(&self.scroll)
            .flex_1()
            .into_any_element()
        };
        v_flex()
            .id("notification-panel")
            .debug_selector(|| "notification-panel".to_owned())
            .key_context(NOTIFICATION_PANEL_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .bg(tokens.colors.background)
            .text_color(tokens.colors.text)
            .text_size(u(tokens.font.body))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.move_selection(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.move_selection(-1, cx)))
            .on_action(cx.listener(|this, _: &OpenSelected, _, cx| {
                let ix = this.selected;
                this.open(ix, cx);
            }))
            .on_action(cx.listener(|this, _: &MarkSelectedRead, _, cx| this.mark_selected_read(cx)))
            .on_action(cx.listener(|this, _: &MarkAllRead, _, cx| this.mark_all_read(cx)))
            .on_action(cx.listener(|this, _: &Close, _, cx| this.close(cx)))
            .child(self.header(cx))
            .child(body)
    }
}

impl NotificationPanel {
    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let centre = self.centre.read(cx);
        let unread = centre.unread();
        let mute = self.active.as_ref().map(|cluster| {
            let muted = centre.is_muted(cluster);
            let verb = if muted { "Unmute" } else { "Mute" };
            (muted, format!("{verb} {}", centre.cluster_label(cluster)))
        });
        let muted_count = centre.muted().len();
        h_flex()
            .id("notification-header")
            .flex_none()
            .flex_wrap()
            .gap(u(tokens.spacing.sm))
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.md))
            .border_b_1()
            .border_color(tokens.colors.border)
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Notifications"),
            )
            .child(
                div()
                    .debug_selector(|| "notification-unread".to_owned())
                    .text_color(tokens.colors.text_muted)
                    .child(format!("{unread} unread")),
            )
            .when(muted_count > 0, |this| {
                this.child(
                    div()
                        .debug_selector(|| "notification-muted-count".to_owned())
                        .text_color(tokens.colors.text_muted)
                        .child(format!("\u{b7} {muted_count} muted")),
                )
            })
            .child(div().flex_1())
            .child(
                div()
                    .debug_selector(|| "notification-mark-all".to_owned())
                    .child(
                        Button::new("notification-mark-all")
                            .label("Mark all read")
                            .xsmall()
                            .ghost()
                            .disabled(unread == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.mark_all_read(cx))),
                    ),
            )
            .when_some(mute, |this, (muted, label)| {
                this.child(
                    div()
                        .debug_selector(|| "notification-mute".to_owned())
                        .child(
                            Button::new("notification-mute")
                                .label(label)
                                .icon(Icon::new(if muted {
                                    IconName::Bell
                                } else {
                                    IconName::EyeOff
                                }))
                                .xsmall()
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_mute(cx))),
                        ),
                )
            })
    }

    fn row(&self, ix: usize, row: &NotificationRow, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let (icon, accent) = level_style(row.level, &colors);
        let selected = ix == self.selected;
        let selector = format!("notification-row-{ix}");
        h_flex()
            .id(("notification-row", ix))
            .debug_selector(move || selector)
            .h(u(px(ROW_HEIGHT)))
            .w_full()
            .items_start()
            .gap(u(tokens.spacing.md))
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.sm))
            .border_b_1()
            .border_color(colors.border_variant)
            .cursor_pointer()
            .when(selected, |this| this.bg(colors.element_selected))
            .hover(|this| this.bg(colors.element_hover))
            .on_click(cx.listener(move |this, _, _, cx| this.open(ix, cx)))
            .child(
                div()
                    .flex_none()
                    .w(u(px(6.)))
                    .h(u(px(6.)))
                    .mt(u(px(6.)))
                    .rounded_full()
                    .when(!row.read, |this| this.bg(colors.accent)),
            )
            .child(Icon::new(icon).size(u(px(14.))).color(accent))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(u(px(2.)))
                    .child(
                        div()
                            .truncate()
                            .when(!row.read, |this| this.font_weight(FontWeight::SEMIBOLD))
                            .child(row.title.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_color(colors.text_muted)
                            .child(row.body.clone()),
                    )
                    .child(
                        h_flex()
                            .gap(u(tokens.spacing.sm))
                            .text_size(u(tokens.font.small))
                            .text_color(colors.text_disabled)
                            .child(div().truncate().child(row.cluster_label.clone()))
                            .child(row.meta.clone()),
                    ),
            )
            .into_any_element()
    }
}
