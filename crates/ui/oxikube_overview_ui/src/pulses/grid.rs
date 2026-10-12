//! The page's layout, shared by the live [`PulsesView`](super::PulsesView) and the preview that
//! the screenshot tests draw (`test_support`): the heading and the grid of tiles.

use gpui::{App, ClickEvent, Div, ParentElement as _, Styled as _, Window, div};
use oxikube_ui::layout::{StyledExt as _, h_flex};
use oxikube_ui::tokens::Tokens;
use oxikube_ui::u;

use super::model::PulsesModel;
use super::specs::COLUMNS;
use super::tile::PulseTile;

/// What a tile does when clicked.
pub(super) type OnClick = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// The hint under the heading.
pub(super) const HINT: &str =
    "arrows or hjkl move · enter opens the table · r counts again · esc closes";

/// The heading: "Pulses" and the keys.
pub(super) fn header(tokens: &Tokens) -> Div {
    h_flex()
        .gap(u(tokens.spacing.lg))
        .items_baseline()
        .child(
            div()
                .text_size(u(tokens.font.heading))
                .font_semibold()
                .child("Pulses"),
        )
        .child(
            div()
                .text_size(u(tokens.font.small))
                .text_color(tokens.colors.text_muted)
                .child(HINT),
        )
}

/// The rows of tiles, [`COLUMNS`] to a row, `model`'s selected tile marked. `click` gives each
/// tile's handler (`None`: not clickable).
pub(super) fn rows(
    model: &mut PulsesModel,
    tokens: &Tokens,
    mut click: impl FnMut(usize) -> Option<OnClick>,
) -> Vec<Div> {
    let selected = model.selected();
    let mut tiles = model
        .tiles_mut()
        .iter_mut()
        .enumerate()
        .map(|(index, tile)| {
            let points = tile.ring.points();
            let tile_element = PulseTile::new(
                index,
                tile.spec.id,
                tile.spec.title,
                tile.face().clone(),
                points,
            )
            .selected(index == selected);
            match click(index) {
                Some(handler) => tile_element.on_click(handler),
                None => tile_element,
            }
        })
        .collect::<Vec<_>>()
        .into_iter();
    let mut out = Vec::new();
    loop {
        let row: Vec<PulseTile> = tiles.by_ref().take(COLUMNS).collect();
        if row.is_empty() {
            break;
        }
        out.push(h_flex().gap(u(tokens.spacing.lg)).children(row));
    }
    out
}
