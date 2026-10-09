//! [`LineCell`]: one line of text filling a table cell, the element of a text cell that fits its
//! column (#605).
//!
//! A cell that fits used to be a `StyledText` inside a flex box that aligned it: two elements, and a
//! text that asks the layout engine to measure it on every frame (a measured leaf, its closure and
//! line wrapper). The table redraws every visible cell whenever it redraws at all (an age moving
//! once a second redraws a screen of rows), so that is per cell, per frame. `LineCell` is one leaf
//! sized by its cell (no measuring: the text is known to fit) that shapes and paints its line when
//! it paints, through the text system's line cache: the same glyphs in the same place (aligned as
//! the column says and centred in the row, as the text in its box was), one element and no measure.

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, Hsla, InspectorElementId, IntoElement,
    LayoutId, Pixels, SharedString, Style, TextAlign, Window, point, relative,
};

use super::column::ColumnAlign;

/// See the [module docs](self).
pub(super) struct LineCell {
    text: SharedString,
    color: Option<Hsla>,
    align: ColumnAlign,
}

impl LineCell {
    /// `text` in `color` (the inherited text colour when `None`), aligned as `align`.
    pub(super) fn new(text: SharedString, color: Option<Hsla>, align: ColumnAlign) -> Self {
        Self { text, color, align }
    }
}

impl IntoElement for LineCell {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for LineCell {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        // The cell's own text style (its size), as a text element in it would read it.
        let style = window.text_style();
        let rem = window.rem_size();
        let font_size = style.font_size.to_pixels(rem);
        let line_height = style.line_height_in_pixels(rem);
        let mut run = style.to_run(self.text.len());
        if let Some(color) = self.color {
            run.color = color;
        }
        let line = window
            .text_system()
            .shape_line(self.text.clone(), font_size, &[run], None);
        // Centred as a flex box centres it: a line taller than the cell overflows it evenly.
        let top = bounds.origin.y + (bounds.size.height - line_height) / 2.;
        // Placed as a text element is in an aligned box: that box is as wide as the line, rounded
        // up to a whole pixel.
        let width = line.width.ceil();
        let left = match self.align {
            ColumnAlign::Left => bounds.origin.x,
            ColumnAlign::Center => bounds.origin.x + (bounds.size.width - width) / 2.,
            ColumnAlign::Right => bounds.origin.x + bounds.size.width - width,
        };
        line.paint(
            point(left, top),
            line_height,
            TextAlign::Left,
            None,
            window,
            cx,
        )
        .ok();
    }
}
