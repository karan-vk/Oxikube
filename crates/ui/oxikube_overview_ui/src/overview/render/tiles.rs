//! The health tiles: nodes ready and not ready, pods by phase. A click opens the kind's list.

use gpui::{AnyElement, Context, IntoElement, ParentElement as _, SharedString, Styled as _};
use oxikube_domain::ids::Gvk;
use oxikube_ui::layout::h_flex;
use oxikube_ui::tile::StatTile;
use oxikube_ui::{ActiveTokens as _, u};

use crate::overview::view::ClusterOverview;

impl ClusterOverview {
    pub(super) fn tiles(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let tiles = self.shown.tiles.iter().map(|tile| {
            let gvk = if tile.id.starts_with("nodes") {
                Gvk::new("", "v1", "Node")
            } else {
                Gvk::new("", "v1", "Pod")
            };
            let id = tile.id;
            StatTile::new(
                SharedString::from(format!("overview-tile-{id}")),
                tile.title,
                tile.value.clone(),
            )
            .caption(tile.caption.clone())
            .tone(tile.tone)
            .hover_text(tile.hover.clone())
            .selector(format!("overview-tile-{id}"))
            .on_click(cx.listener(move |this, _, _, cx| this.open_list(gvk.clone(), cx)))
        });
        h_flex()
            .flex_wrap()
            .gap(u(tokens.spacing.lg))
            .children(tiles)
            .into_any_element()
    }
}
