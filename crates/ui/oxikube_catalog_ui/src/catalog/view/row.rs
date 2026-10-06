//! One row of the catalog list.

use gpui::{
    App, Context, Div, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    Pixels, SharedString, StatefulInteractiveElement as _, Styled as _, div, px,
};
use jiff::Timestamp;
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, u};

use super::CatalogView;
use crate::catalog::model::{Badge, Tone};

/// The height of every row, at 100 % zoom: uniform, as `uniform_list` needs. Tall enough for a
/// name and, under it, the reason a cluster is invalid or failing.
pub(super) const ROW_HEIGHT: Pixels = px(44.);

/// Width of the favourite star column.
const STAR_WIDTH: f32 = 28.;
/// Width of the status badge column.
const STATUS_WIDTH: f32 = 116.;
/// Width of the last-used column.
const LAST_USED_WIDTH: f32 = 88.;

/// How a column takes its width: a fixed size or a share of what is left.
#[derive(Clone, Copy)]
pub(super) enum Width {
    Fixed(f32),
    Flex(f32),
}

/// One column of the list: its head and its width.
#[derive(Clone, Copy)]
pub(super) struct Column {
    pub(super) title: &'static str,
    pub(super) width: Width,
}

impl Column {
    /// An empty, sized cell of this column. Flexible cells share the free width by their
    /// weight and clip their content.
    pub(super) fn frame(&self) -> Div {
        let cell = div()
            .h_full()
            .flex()
            .items_center()
            .overflow_hidden()
            .pr(u(px(8.)));
        match self.width {
            Width::Fixed(width) => cell.w(u(px(width))).flex_none(),
            Width::Flex(weight) => {
                let mut cell = cell.flex_shrink(1.).flex_basis(u(px(0.))).min_w(u(px(48.)));
                cell.style().flex_grow = Some(weight);
                cell
            }
        }
    }

    /// A text cell of this column.
    pub(super) fn text(&self, content: impl Into<SharedString>) -> Div {
        self.frame().child(div().truncate().child(content.into()))
    }
}

/// The columns, left to right: favourite star, the facts, the status and the last-used time.
pub(super) const COLUMNS: [Column; 7] = [
    Column {
        title: "",
        width: Width::Fixed(STAR_WIDTH),
    },
    Column {
        title: "Name",
        width: Width::Flex(2.0),
    },
    Column {
        title: "Cluster",
        width: Width::Flex(1.5),
    },
    Column {
        title: "User",
        width: Width::Flex(1.5),
    },
    Column {
        title: "Source",
        width: Width::Flex(2.0),
    },
    Column {
        title: "Status",
        width: Width::Fixed(STATUS_WIDTH),
    },
    Column {
        title: "Last used",
        width: Width::Fixed(LAST_USED_WIDTH),
    },
];

const NAME: usize = 1;
const CLUSTER: usize = 2;
const USER: usize = 3;
const SOURCE: usize = 4;
const LAST_USED: usize = 6;

/// "just now", "5m ago", "3h ago", "2d ago", "3mo ago", "2y ago" or "never": coarse on
/// purpose, a catalog does not need seconds.
pub(crate) fn last_used_text(last_used: Option<Timestamp>, now: Timestamp) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    let Some(at) = last_used else {
        return "never".to_owned();
    };
    // A clock that ran backwards (or a time in the future) reads as "just now", never negative.
    let secs = now.duration_since(at).as_secs().max(0);
    if secs < MINUTE {
        "just now".to_owned()
    } else if secs < HOUR {
        format!("{}m ago", secs / MINUTE)
    } else if secs < DAY {
        format!("{}h ago", secs / HOUR)
    } else if secs < 30 * DAY {
        format!("{}d ago", secs / DAY)
    } else if secs < 365 * DAY {
        format!("{}mo ago", secs / (30 * DAY))
    } else {
        format!("{}y ago", secs / (365 * DAY))
    }
}

/// The colour a status [`Tone`] is drawn in.
pub(crate) fn tone_colour(tone: Tone, cx: &App) -> gpui::Hsla {
    let colors = cx.colors();
    match tone {
        Tone::Muted => colors.text_muted,
        Tone::Info => colors.info,
        Tone::Success => colors.success,
        Tone::Warning => colors.warning,
        Tone::Error => colors.error,
    }
}

impl CatalogView {
    pub(super) fn render_row(&mut self, ix: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let Some(row) = self.model.row(ix) else {
            return div().into_any_element();
        };
        let badge = self.model.badge(ix).expect("the row exists");
        let cluster = row.entry().id().clone();
        let selected = self.model.selected() == Some(&cluster);
        let favourite = row.entry().favourite;
        let last_used = last_used_text(row.entry().last_used, self.deps.clock.now());
        let (name, cluster_name, user, source) = (
            row.name().clone(),
            row.cluster().clone(),
            row.user().clone(),
            row.source().clone(),
        );

        let star = div()
            .id(("catalog-star", ix))
            .debug_selector(move || format!("catalog-star-{ix}"))
            .h_full()
            .w(u(px(STAR_WIDTH)))
            .flex_none()
            .flex()
            .items_center()
            .cursor_pointer()
            // The row below connects on press; the star must not.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_favourite(cluster.clone(), cx);
            }))
            .child(
                Icon::new(IconName::Star)
                    .size(u(px(14.)))
                    .color(if favourite {
                        colors.warning
                    } else {
                        colors.text_disabled
                    }),
            );

        // The name, and under it why the cluster is invalid or failing, when it is.
        let name_cell = {
            let mut lines = v_flex().min_w_0().child(
                div()
                    .truncate()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(name),
            );
            if let Some(detail) = badge
                .detail
                .as_deref()
                .filter(|_| badge.tone != Tone::Muted)
            {
                lines = lines.child(
                    div()
                        .truncate()
                        .text_size(u(tokens.font.small))
                        .text_color(tone_colour(badge.tone, cx))
                        .child(detail.to_owned()),
                );
            }
            COLUMNS[NAME].frame().child(lines)
        };

        let mut row = h_flex()
            .id(("catalog-row", ix))
            .debug_selector(move || format!("catalog-row-{ix}"))
            .h(u(ROW_HEIGHT))
            .w_full()
            .flex_none()
            .px(u(tokens.spacing.xl))
            .items_center()
            .cursor_pointer()
            .hover(|style| style.bg(colors.element_hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| this.activate_row(ix, cx)),
            );
        if selected {
            row = row.bg(colors.element_selected);
        }
        row.child(star)
            .child(name_cell)
            .child(
                COLUMNS[CLUSTER]
                    .text(cluster_name)
                    .text_color(colors.text_muted),
            )
            .child(COLUMNS[USER].text(user).text_color(colors.text_muted))
            .child(COLUMNS[SOURCE].text(source).text_color(colors.text_muted))
            .child(self.status_cell(ix, &badge, cx))
            .child(
                COLUMNS[LAST_USED]
                    .text(last_used)
                    .text_color(colors.text_muted),
            )
            .into_any_element()
    }

    fn status_cell(&self, ix: usize, badge: &Badge, cx: &App) -> impl IntoElement {
        let colour = tone_colour(badge.tone, cx);
        let tokens = cx.tokens();
        div()
            .h_full()
            .w(u(px(STATUS_WIDTH)))
            .flex_none()
            .flex()
            .items_center()
            .child(
                h_flex()
                    .debug_selector(move || format!("catalog-status-{ix}"))
                    .gap(u(tokens.spacing.sm))
                    .items_center()
                    .px(u(tokens.spacing.md))
                    .py(u(tokens.spacing.xs))
                    .rounded(u(tokens.radius.sm))
                    .text_size(u(tokens.font.small))
                    .text_color(colour)
                    .bg(colour.opacity(0.12))
                    .child(div().size(u(px(6.))).flex_none().rounded_full().bg(colour))
                    .child(div().truncate().child(badge.label)),
            )
    }
}
