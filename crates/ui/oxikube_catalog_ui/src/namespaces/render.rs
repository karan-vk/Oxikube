//! Drawing the selector: the trigger, and the dropdown with its search box and virtualised
//! list.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window, deferred,
    div, prelude::FluentBuilder as _, px, uniform_list,
};
use oxikube_app::session::namespaces::NamespaceSource;
use oxikube_ui::{
    ActiveTokens as _, Icon, IconName,
    input::Input,
    layout::{h_flex, v_flex},
    u,
};

use super::actions::{
    Close, FocusList, FocusSearch, LIST_CONTEXT, MoveDown, MoveUp, Open, SEARCH_CONTEXT,
    SELECTOR_CONTEXT, SelectSlot, ToggleFavouriteHighlighted, ToggleHighlighted,
};
use super::model::{NamespaceRow, Row};
use super::selector::NamespaceSelector;

/// Row height in unscaled pixels; the list is uniform.
const ROW_HEIGHT: f32 = 28.;
/// Most rows shown before the list scrolls.
const MAX_VISIBLE_ROWS: usize = 10;
const WIDTH: f32 = 300.;

impl Render for NamespaceSelector {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The trigger is the first child: remember where it is, so a press on it is not taken
        // for a press outside the dropdown.
        let trigger_bounds = self.trigger_bounds.clone();
        let mut root = div()
            .on_children_prepainted(move |bounds, _, _| {
                if let Some(trigger) = bounds.first() {
                    trigger_bounds.set(*trigger);
                }
            })
            .id("namespace-selector")
            .relative()
            .on_action(cx.listener(|this, _: &Open, window, cx| this.open(window, cx)))
            .on_action(cx.listener(|this, _: &Close, window, cx| this.close(window, cx)))
            .on_action(
                cx.listener(|this, action: &SelectSlot, _, cx| this.select_slot(action.slot, cx)),
            )
            .on_action(cx.listener(|this, _: &MoveUp, _, cx| this.move_highlight(-1, cx)))
            .on_action(cx.listener(|this, _: &MoveDown, _, cx| this.move_highlight(1, cx)))
            .on_action(
                cx.listener(|this, _: &ToggleHighlighted, _, cx| {
                    this.activate(this.highlighted, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ToggleFavouriteHighlighted, _, cx| {
                if let Some(name) = this
                    .rows
                    .get(this.highlighted)
                    .and_then(Row::namespace)
                    .map(str::to_owned)
                {
                    this.toggle_favourite(&name, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.search.update(cx, |input, cx| input.focus(window, cx));
            }))
            .on_action(cx.listener(|this, _: &FocusList, window, cx| {
                window.focus(&this.list_focus, cx);
            }))
            .child(self.render_trigger(cx));
        if self.open {
            root = root.child(deferred(self.render_dropdown(cx)).with_priority(2));
        }
        root
    }
}

impl NamespaceSelector {
    fn render_trigger(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        h_flex()
            .id("namespace-trigger")
            .debug_selector(|| "namespace-trigger".to_owned())
            .key_context(SELECTOR_CONTEXT)
            .track_focus(&self.trigger_focus)
            .gap(u(tokens.spacing.sm))
            .items_center()
            .h(u(px(26.)))
            .px(u(tokens.spacing.md))
            .rounded(u(tokens.radius.md))
            .border_1()
            .border_color(if self.open {
                colors.border_focused
            } else {
                colors.border
            })
            .bg(colors.element)
            .text_color(colors.text)
            .text_size(u(tokens.font.body))
            .cursor_pointer()
            .hover(|style| style.bg(colors.element_hover))
            .child(
                Icon::new(IconName::Layers)
                    .size(u(px(14.)))
                    .color(colors.text_muted),
            )
            .child(div().child(self.label()))
            .child(
                Icon::new(IconName::ChevronDown)
                    .size(u(px(12.)))
                    .color(colors.text_muted),
            )
            .on_click(cx.listener(|this, _, window, cx| this.toggle_open(window, cx)))
    }

    fn render_dropdown(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let visible = self.rows.len().clamp(1, MAX_VISIBLE_ROWS);
        let list = uniform_list(
            "namespace-rows",
            self.rows.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _window, cx| {
                range
                    .map(|ix| this.render_row(ix, cx))
                    .collect::<Vec<AnyElement>>()
            }),
        )
        .track_scroll(&self.scroll)
        .h(u(px(ROW_HEIGHT * visible as f32)));

        v_flex()
            .id("namespace-dropdown")
            .debug_selector(|| "namespace-dropdown".to_owned())
            .absolute()
            .top(u(px(30.)))
            .left_0()
            .w(u(px(WIDTH)))
            .gap(u(tokens.spacing.sm))
            .p(u(tokens.spacing.sm))
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .text_size(u(tokens.font.body))
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, window, cx| {
                // A press on the trigger is its click's business (it toggles); closing here
                // would make the click reopen the dropdown.
                let on_trigger = event.button == MouseButton::Left
                    && this.trigger_bounds.get().contains(&event.position);
                if !on_trigger {
                    this.close(window, cx);
                }
            }))
            .child(
                div()
                    .key_context(SEARCH_CONTEXT)
                    .debug_selector(|| "namespace-search".to_owned())
                    .child(Input::new(&self.search)),
            )
            .child(
                div()
                    .id("namespace-list")
                    .key_context(LIST_CONTEXT)
                    .track_focus(&self.list_focus)
                    .child(list),
            )
            .child(self.render_notice(cx))
    }

    /// The line under the list: why it is short, or what went wrong.
    fn render_notice(&self, cx: &Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let (text, colour) = if let Some(error) = &self.error {
            (error.to_string(), colors.error)
        } else {
            match self.catalog.source {
                NamespaceSource::Forbidden => (
                    "This cluster does not let you list namespaces. Type a name to add it."
                        .to_owned(),
                    colors.warning,
                ),
                NamespaceSource::Unavailable if self.catalog.names.is_empty() => (
                    "Namespaces are not available yet. Type a name to add it.".to_owned(),
                    colors.text_muted,
                ),
                _ => (
                    "0 all  1-9 favourites  f pin  / search".to_owned(),
                    colors.text_muted,
                ),
            }
        };
        div()
            .id("namespace-notice")
            .debug_selector(|| "namespace-notice".to_owned())
            .px(u(tokens.spacing.sm))
            .text_size(u(tokens.font.small))
            .text_color(colour)
            .child(text)
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let Some(row) = self.rows.get(ix) else {
            return div().into_any_element();
        };
        let base = h_flex()
            .id(("namespace-row", ix))
            .w_full()
            .h(u(px(ROW_HEIGHT)))
            .px(u(tokens.spacing.sm))
            .gap(u(tokens.spacing.sm))
            .items_center()
            .rounded(u(tokens.radius.sm));
        let check = |checked: bool| {
            div().w(u(px(14.))).flex_none().children(checked.then(|| {
                Icon::new(IconName::Check)
                    .size(u(px(14.)))
                    .color(colors.accent)
            }))
        };
        let digit = |slot: u8| {
            div()
                .flex_none()
                .px(u(tokens.spacing.sm))
                .rounded(u(tokens.radius.sm))
                .bg(colors.element)
                .text_size(u(tokens.font.small))
                .text_color(colors.text_muted)
                .child(slot.to_string())
        };
        match row {
            Row::Header(title) => base
                .text_size(u(tokens.font.small))
                .text_color(colors.text_muted)
                .child(title.clone())
                .into_any_element(),
            Row::All { checked } => base
                .debug_selector(|| "namespace-row-all".to_owned())
                .when(ix == self.highlighted, |row| row.bg(colors.element_hover))
                .child(check(*checked))
                .child(div().flex_1().child("All namespaces"))
                .child(digit(0))
                .on_click(cx.listener(move |this, _, _, cx| this.activate(ix, cx)))
                .into_any_element(),
            Row::Add { name } => base
                .debug_selector(|| "namespace-row-add".to_owned())
                .when(ix == self.highlighted, |row| row.bg(colors.element_hover))
                .child(
                    Icon::new(IconName::Plus)
                        .size(u(px(14.)))
                        .color(colors.text_muted),
                )
                .child(div().flex_1().child(format!("Add \"{name}\"")))
                .on_click(cx.listener(move |this, _, _, cx| this.activate(ix, cx)))
                .into_any_element(),
            Row::Namespace(NamespaceRow {
                name,
                checked,
                favourite,
                slot,
            }) => {
                let star_name = name.clone();
                let star = div()
                    .id(("namespace-star", ix))
                    .debug_selector(move || format!("namespace-star-{star_name}"))
                    .flex_none()
                    .cursor_pointer()
                    .child(
                        Icon::new(IconName::Star)
                            .size(u(px(14.)))
                            .color(if *favourite {
                                colors.warning
                            } else {
                                colors.text_disabled
                            }),
                    )
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click({
                        let name = name.clone();
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_favourite(&name, cx)
                        })
                    });
                let label = name.clone();
                base.debug_selector({
                    let name = name.clone();
                    move || format!("namespace-row-{name}")
                })
                .when(ix == self.highlighted, |row| row.bg(colors.element_hover))
                .child(check(*checked))
                .child(div().flex_1().overflow_hidden().child(label))
                .children(slot.map(digit))
                .child(star)
                .on_click(cx.listener(move |this, _, _, cx| this.activate(ix, cx)))
                .into_any_element()
            }
        }
    }
}
