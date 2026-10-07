//! [`StatTile`]: a clickable number with a caption, the building block of overview pages.
//!
//! One look for every tile (title on top, a big value, a caption underneath), coloured from the
//! theme's status tokens by [`TileTone`], zoom-safe, with an optional hover text. Plain `div`s:
//! nothing here depends on gpui-component.

use gpui::{
    App, ClickEvent, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::layout::{StyledExt as _, v_flex};
use crate::tokens::ActiveTokens as _;
use crate::tooltip::Tooltip;
use crate::u;

/// How a tile's value reads: which status colour it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TileTone {
    /// Plain text colour (a count with nothing to judge).
    #[default]
    Neutral,
    /// Everything is fine (the success colour).
    Good,
    /// Something needs attention (the warning colour).
    Warn,
    /// Not a number: loading, not counted (the muted colour).
    Muted,
}

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// A titled, clickable stat. Build it per frame from a model; it keeps no state.
#[derive(IntoElement)]
pub struct StatTile {
    id: ElementId,
    title: SharedString,
    value: SharedString,
    caption: SharedString,
    tone: TileTone,
    hover: Option<SharedString>,
    selector: Option<String>,
    on_click: Option<ClickHandler>,
}

impl StatTile {
    /// A tile with `title` and `value`.
    pub fn new(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        value: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            value: value.into(),
            caption: SharedString::default(),
            tone: TileTone::Neutral,
            hover: None,
            selector: None,
            on_click: None,
        }
    }

    /// The line under the value ("5 healthy").
    #[must_use]
    pub fn caption(mut self, caption: impl Into<SharedString>) -> Self {
        self.caption = caption.into();
        self
    }

    /// The value's colour.
    #[must_use]
    pub fn tone(mut self, tone: TileTone) -> Self {
        self.tone = tone;
        self
    }

    /// Text shown after hovering the tile for a moment.
    #[must_use]
    pub fn hover_text(mut self, text: Option<impl Into<SharedString>>) -> Self {
        self.hover = text.map(Into::into);
        self
    }

    /// A test selector (`debug_selector`) for the tile.
    #[must_use]
    pub fn selector(mut self, selector: impl Into<String>) -> Self {
        self.selector = Some(selector.into());
        self
    }

    /// Runs `handler` on click; the tile then shows a pointer and a hover background.
    #[must_use]
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for StatTile {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let value_colour = match self.tone {
            TileTone::Neutral => colors.text,
            TileTone::Good => colors.success,
            TileTone::Warn => colors.warning,
            TileTone::Muted => colors.text_muted,
        };
        let selector = self.selector.clone();
        let hover = self.hover.clone();
        let tile = v_flex()
            .id(self.id)
            .when_some(selector, |tile, selector| {
                tile.debug_selector(move || selector)
            })
            .w(u(px(176.)))
            .flex_none()
            .gap(u(tokens.spacing.sm))
            .p(u(tokens.spacing.lg))
            .rounded(u(tokens.radius.md))
            .border_1()
            .border_color(colors.border_variant)
            .bg(colors.surface)
            .child(
                div()
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .truncate()
                    .child(self.title),
            )
            .child(
                div()
                    .text_size(u(px(26.)))
                    .text_color(value_colour)
                    .font_semibold()
                    .truncate()
                    .child(self.value),
            )
            .child(
                div()
                    .h(u(px(16.)))
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .truncate()
                    .child(self.caption),
            );
        let tile = match self.on_click {
            Some(handler) => tile
                .cursor_pointer()
                .hover(move |style| style.bg(colors.element_hover))
                .on_click(move |event, window, cx| handler(event, window, cx)),
            None => tile,
        };
        match hover {
            Some(text) => tile
                .tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
                .into_any_element(),
            None => tile.into_any_element(),
        }
    }
}
