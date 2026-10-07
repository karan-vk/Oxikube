//! Drawing the sidebar: one virtualised list of uniform rows.

use gpui::{
    AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px, uniform_list,
};
use oxikube_app::CountState;
use oxikube_ui::{
    ActiveTokens as _, Icon, IconName,
    layout::{h_flex, v_flex},
    tooltip::Tooltip,
    u,
};

use super::SidebarPanel;
use crate::sidebar::actions::{Activate, Collapse, Expand, MoveDown, MoveUp, SIDEBAR_CONTEXT};
use crate::sidebar::badges::badge_text;
use crate::sidebar::rows::{EntryRow, GroupRow, NoticeKind, NoticeRow, Row, SectionRow};

/// Row height in unscaled pixels; the list is uniform.
const ROW_HEIGHT: f32 = 28.;
/// Indentation per depth level, unscaled pixels.
const INDENT: f32 = 16.;

impl Render for SidebarPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        let list = uniform_list(
            "cluster-sidebar-rows",
            self.rows.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _window, cx| {
                range
                    .map(|ix| this.render_row(ix, cx))
                    .collect::<Vec<AnyElement>>()
            }),
        )
        .track_scroll(&self.scroll)
        .size_full();

        v_flex()
            .id("cluster-sidebar")
            .debug_selector(|| "cluster-sidebar".to_owned())
            .key_context(SIDEBAR_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &MoveUp, _, cx| this.move_highlight(-1, cx)))
            .on_action(cx.listener(|this, _: &MoveDown, _, cx| this.move_highlight(1, cx)))
            .on_action(cx.listener(|this, _: &Activate, _, cx| {
                if let Some(id) = this.highlighted().map(str::to_owned) {
                    this.activate(&id, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Collapse, _, cx| {
                if let Some(id) = this.highlighted().map(str::to_owned) {
                    this.set_open(&id, false, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Expand, _, cx| {
                if let Some(id) = this.highlighted().map(str::to_owned) {
                    this.set_open(&id, true, cx);
                }
            }))
            .size_full()
            .bg(colors.surface)
            .text_color(colors.text)
            .child(div().flex_1().min_h_0().child(list))
    }
}

impl SidebarPanel {
    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        match self.rows.get(ix) {
            Some(Row::Section(row)) => self.render_section(ix, row, cx),
            Some(Row::Group(row)) => self.render_group(ix, row, cx),
            Some(Row::Entry(row)) => self.render_entry(ix, row, cx),
            Some(Row::Notice(row)) => render_notice(ix, row, cx),
            None => div().into_any_element(),
        }
    }

    /// The shared frame of an interactive row: height, padding, hover and highlight.
    fn row_frame(
        &self,
        ix: usize,
        id: &SharedString,
        indent: f32,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let highlighted = self.highlighted.as_ref() == Some(id);
        let selected = self.selected.as_ref() == Some(id);
        let hover_id = id.clone();
        let click_id = id.clone();
        h_flex()
            .id(("cluster-sidebar-row", ix))
            .w_full()
            .h(u(px(ROW_HEIGHT)))
            .pl(u(tokens.spacing.md + px(indent)))
            .pr(u(tokens.spacing.md))
            .gap(u(tokens.spacing.sm))
            .items_center()
            .cursor_pointer()
            .text_size(u(tokens.font.body))
            .when(highlighted, |row| row.bg(colors.element_hover))
            .when(selected, |row| row.bg(colors.element_selected))
            .on_hover(cx.listener(move |this, hovered, _, cx| {
                if *hovered {
                    this.hover(hover_id.clone(), cx);
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.focus, cx);
                this.activate(&click_id, cx);
            }))
    }

    fn render_section(&self, ix: usize, row: &SectionRow, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let id = row.id.clone();
        self.row_frame(ix, &id, 0., cx)
            .debug_selector(move || format!("sidebar-section-{id}"))
            .child(disclosure(row.expandable, row.open, cx))
            .child(
                Icon::new(row.icon)
                    .size(u(px(14.)))
                    .color(colors.text_muted),
            )
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(row.title.clone()),
            )
            .child(
                match row.count.as_ref().filter(|s| badge_text(s).is_some()) {
                    Some(state) => count_badge(&row.id, state, cx),
                    None => count_placeholder(&row.id, None, cx).into_any_element(),
                },
            )
            .into_any_element()
    }

    fn render_group(&self, ix: usize, row: &GroupRow, cx: &mut Context<Self>) -> AnyElement {
        let id = row.id.clone();
        self.row_frame(ix, &id, INDENT, cx)
            .debug_selector(move || format!("sidebar-group-{id}"))
            .child(disclosure(true, row.open, cx))
            .child(div().flex_1().truncate().child(row.title.clone()))
            .child(count_placeholder(&row.id, row.count, cx))
            .into_any_element()
    }

    fn render_entry(&self, ix: usize, row: &EntryRow, cx: &mut Context<Self>) -> AnyElement {
        let id = row.id.clone();
        let indent = INDENT * f32::from(row.depth) + 14.;
        self.row_frame(ix, &id, indent, cx)
            .debug_selector(move || format!("sidebar-entry-{id}"))
            .child(div().flex_1().truncate().child(row.title.clone()))
            .when_some(row.count.as_ref(), |entry, state| {
                entry.child(count_badge(&row.id, state, cx))
            })
            .into_any_element()
    }
}

/// The count badge of an entry (E07-S11): the total, tinted when some are unhealthy; "no access"
/// when the kind is forbidden; a dash with the reason on hover when it is not counted.
fn count_badge(
    id: &SharedString,
    state: &CountState,
    cx: &mut Context<SidebarPanel>,
) -> AnyElement {
    let Some((text, hover)) = badge_text(state) else {
        return div().into_any_element();
    };
    let tokens = cx.tokens();
    let unhealthy = state.count().is_some_and(|c| !c.all_healthy());
    let colour = if unhealthy || state.is_no_access() {
        tokens.colors.warning
    } else {
        tokens.colors.text_muted
    };
    let selector = format!("sidebar-badge-{id}");
    let badge = div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector)
        .flex_none()
        .text_size(u(tokens.font.small))
        .text_color(colour)
        .child(text);
    match hover {
        Some(hover) => badge
            .tooltip(move |window, cx| Tooltip::new(hover.clone()).build(window, cx))
            .into_any_element(),
        None => badge.into_any_element(),
    }
}

/// The chevron of a section or group; an empty slot of the same width when there is nothing to
/// open, so titles line up.
fn disclosure(expandable: bool, open: bool, cx: &mut Context<SidebarPanel>) -> impl IntoElement {
    let colors = cx.colors();
    div().w(u(px(14.))).flex_none().when(expandable, |slot| {
        slot.child(
            Icon::new(if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .size(u(px(14.)))
            .color(colors.text_muted),
        )
    })
}

/// The count badge: the number when the `ResourceStore` has one, a muted dash until then.
fn count_placeholder(
    id: &SharedString,
    count: Option<usize>,
    cx: &mut Context<SidebarPanel>,
) -> impl IntoElement {
    let tokens = cx.tokens();
    let id = id.clone();
    div()
        .flex_none()
        .debug_selector(move || format!("sidebar-count-{id}"))
        .text_size(u(tokens.font.small))
        .text_color(tokens.colors.text_disabled)
        .child(match count {
            Some(count) => count.to_string(),
            None => "–".to_owned(),
        })
}

fn render_notice(ix: usize, row: &NoticeRow, cx: &mut Context<SidebarPanel>) -> AnyElement {
    let tokens = cx.tokens();
    let colour = match row.kind {
        NoticeKind::Muted => tokens.colors.text_muted,
        NoticeKind::Warning => tokens.colors.warning,
    };
    let id = row.id;
    h_flex()
        .id(("cluster-sidebar-row", ix))
        .debug_selector(move || format!("sidebar-notice-{id}"))
        .w_full()
        .h(u(px(ROW_HEIGHT)))
        .px(u(tokens.spacing.md))
        .items_center()
        .text_size(u(tokens.font.small))
        .text_color(colour)
        .child(div().truncate().child(row.text.clone()))
        .into_any_element()
}
