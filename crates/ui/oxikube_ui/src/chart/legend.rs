//! The legend: one swatch and name per series; a click switches the series off and on.

use super::data::Series;
use super::palette::ChartPalette;
use super::spec::Spec;
use super::view::Chart;
use crate::layout::h_flex;
use crate::tokens::Tokens;
use crate::u;
use gpui::{
    Context, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
    px,
};

/// The legend row for `spec`'s series. Hidden series stay listed (dimmed, hollow swatch) so they
/// can be switched back on.
pub(super) fn legend(
    id: &ElementId,
    spec: &Spec,
    palette: &ChartPalette,
    tokens: &Tokens,
    cx: &mut Context<Chart>,
) -> impl IntoElement {
    let colors = tokens.colors;
    h_flex()
        .flex_none()
        .flex_wrap()
        .gap_x(u(tokens.spacing.lg))
        .gap_y(u(tokens.spacing.xs))
        .px(u(tokens.spacing.sm))
        .pb(u(tokens.spacing.sm))
        .children(spec.series.iter().enumerate().map(|(index, series)| {
            let hidden = spec.is_hidden(&series.name);
            let color = palette.resolve(series.style.color, index);
            entry(id, index, series, hidden)
                .text_size(u(tokens.font.small))
                .text_color(if hidden {
                    colors.text_disabled
                } else {
                    colors.text_muted
                })
                .hover(move |style| style.text_color(colors.text))
                .child(
                    div()
                        .size(u(px(8.)))
                        .rounded(u(px(2.)))
                        .border_1()
                        .border_color(color)
                        .when(!hidden, |swatch| swatch.bg(color)),
                )
                .child(series.name.clone())
                .on_click({
                    let name = series.name.clone();
                    cx.listener(move |chart, _, _, cx| chart.toggle_series(&name, cx))
                })
        }))
}

fn entry(id: &ElementId, index: usize, series: &Series, hidden: bool) -> gpui::Stateful<gpui::Div> {
    let label: SharedString = format!(
        "{}, {}",
        series.name,
        if hidden {
            "hidden: activate to show"
        } else {
            "shown: activate to hide"
        }
    )
    .into();
    h_flex()
        .id(ElementId::NamedInteger(
            SharedString::from(format!("{id}-legend")),
            index as u64,
        ))
        .gap(px(5.))
        .items_center()
        .debug_selector(move || format!("chart-legend-{index}"))
        .cursor_pointer()
        .role(Role::Button)
        .aria_label(label)
}
