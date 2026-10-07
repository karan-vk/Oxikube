//! The toolbar: the container selector, the range presets (tail, head, 1m ... 1h), and the
//! previous / wrap / timestamps / autoscroll / fullscreen toggles. Every control sends its
//! `logs::*` command, like the keys.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    div, px,
};
use oxikube_domain::log::{LevelChip, LogRange};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, Selectable as _, h_flex};
use oxikube_ui::menu::{DropdownMenu as _, PopupMenuItem};
use oxikube_ui::{ActiveTokens as _, Colors, Sizable as _, u};

use super::LogView;

impl LogView {
    pub(crate) fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        h_flex()
            .id("log-toolbar")
            .flex_none()
            .gap(u(tokens.spacing.sm))
            .px(u(tokens.spacing.md))
            .py(u(tokens.spacing.sm))
            .items_center()
            .border_b_1()
            .border_color(tokens.colors.border_variant)
            .child(
                div()
                    .text_color(tokens.colors.text_muted)
                    .text_size(u(tokens.font.small))
                    .child(self.title()),
            )
            .child(self.container_selector(cx))
            .child(div().w(u(px(8.))))
            .children(LogRange::ALL.map(|range| self.range_button(range, cx)))
            .child(div().flex_1())
            .child(self.toggle(
                "log-find",
                "Search",
                self.search.state.is_open(),
                cx,
                |view, cx| view.request_find(cx),
            ))
            .child(self.toggle(
                "log-previous",
                "Previous",
                self.options.previous,
                cx,
                |view, cx| view.request_previous(cx),
            ))
            .child(
                self.toggle("log-wrap", "Wrap", self.options.wrap, cx, |view, cx| {
                    view.request_wrap(cx)
                }),
            )
            .child(self.toggle(
                "log-timestamps",
                "Timestamps",
                self.options.timestamps,
                cx,
                |view, cx| view.request_timestamps(cx),
            ))
            .children(self.shows_json_controls().then(|| {
                self.toggle("log-json", "JSON", self.options.json, cx, |view, cx| {
                    view.request_json_mode(cx)
                })
            }))
            .child(self.toggle(
                "log-autoscroll",
                "Autoscroll",
                self.follow.is_on(),
                cx,
                |view, cx| view.request_autoscroll(cx),
            ))
            .child(self.toggle(
                "log-fullscreen",
                "Fullscreen",
                self.is_fullscreen(cx),
                cx,
                |view, cx| view.request_fullscreen(cx),
            ))
    }

    /// The level chips (JSON mode): one per level plus `text`, each showing or hiding its lines.
    pub(crate) fn level_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.options.json || !self.shows_json_controls() {
            return None;
        }
        let tokens = cx.tokens();
        let chips = LevelChip::ALL.map(|chip| {
            let id = format!("log-level-{}", chip.label());
            let selector = id.clone();
            let colour = chip_colour(chip, &tokens.colors);
            h_flex()
                .gap(u(tokens.spacing.xs))
                .items_center()
                .debug_selector(move || selector)
                .child(
                    div()
                        .size(u(px(6.)))
                        .rounded_full()
                        .bg(if self.levels.shows(chip) {
                            colour
                        } else {
                            tokens.colors.text_disabled
                        }),
                )
                .child(
                    Button::new(gpui::SharedString::from(id))
                        .label(chip.label())
                        .ghost()
                        .xsmall()
                        .selected(self.levels.shows(chip))
                        .on_click(cx.listener(move |view, _, _, cx| view.request_level(chip, cx))),
                )
        });
        Some(
            h_flex()
                .id("log-levels")
                .debug_selector(|| "log-levels".into())
                .flex_none()
                .gap(u(tokens.spacing.sm))
                .px(u(tokens.spacing.md))
                .py(u(tokens.spacing.xs))
                .items_center()
                .border_b_1()
                .border_color(tokens.colors.border_variant)
                .child(
                    div()
                        .text_color(tokens.colors.text_muted)
                        .text_size(u(tokens.font.small))
                        .child("Levels"),
                )
                .children(chips)
                .into_any_element(),
        )
    }

    /// `namespace/pod`.
    fn title(&self) -> String {
        match &self.target.namespace {
            Some(namespace) => format!("{namespace}/{}", self.target.name),
            None => self.target.name.to_string(),
        }
    }

    fn container_selector(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.options.container.clone();
        let label = current
            .clone()
            .unwrap_or_else(|| "default container".to_owned());
        let choices: Vec<(String, String)> = self
            .containers
            .iter()
            .map(|c| (c.name.to_string(), c.label()))
            .collect();
        let view = cx.entity().downgrade();
        let button = Button::new("log-container")
            .label(label)
            .ghost()
            .xsmall()
            .disabled(choices.is_empty())
            .dropdown_menu(move |mut menu, _, _| {
                for (name, label) in &choices {
                    let (view, name) = (view.clone(), name.clone());
                    let checked = current.as_deref() == Some(name.as_str());
                    menu = menu.item(PopupMenuItem::new(label.clone()).checked(checked).on_click(
                        move |_, _, cx| {
                            view.update(cx, |view, cx| view.request_container(&name, cx))
                                .ok();
                        },
                    ));
                }
                menu
            });
        div()
            .debug_selector(|| "log-container".into())
            .child(button)
            .into_any_element()
    }

    fn range_button(&self, range: LogRange, cx: &mut Context<Self>) -> AnyElement {
        let id = format!("log-range-{}", range.label());
        let selector = id.clone();
        div()
            .debug_selector(move || selector)
            .child(
                Button::new(gpui::SharedString::from(id))
                    .label(range.label())
                    .ghost()
                    .xsmall()
                    .selected(self.options.range == range)
                    .on_click(cx.listener(move |view, _, _, cx| view.request_range(range, cx))),
            )
            .into_any_element()
    }

    fn toggle(
        &self,
        id: &'static str,
        label: &'static str,
        on: bool,
        cx: &mut Context<Self>,
        request: fn(&mut LogView, &mut Context<LogView>),
    ) -> AnyElement {
        div()
            .debug_selector(move || id.to_owned())
            .child(
                Button::new(id)
                    .label(label)
                    .ghost()
                    .xsmall()
                    .selected(on)
                    .on_click(cx.listener(move |view, _, _, cx| request(view, cx))),
            )
            .into_any_element()
    }
}

/// The dot of a chip: the level's accent (the `text` chip is neutral).
fn chip_colour(chip: LevelChip, colors: &Colors) -> gpui::Hsla {
    match chip {
        LevelChip::Trace | LevelChip::Debug => colors.text_muted,
        LevelChip::Info => colors.info,
        LevelChip::Warn => colors.warning,
        LevelChip::Error | LevelChip::Fatal => colors.error,
        LevelChip::Text => colors.text,
    }
}
