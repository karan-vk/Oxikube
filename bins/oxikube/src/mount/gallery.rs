//! The chart gallery as a workspace tab (E13-S07): every state of the `oxikube_ui` charts on one
//! page, opened by `charts::OpenGallery` ("Open Chart Gallery" in the palette) or
//! `view::Open { view: "chart-gallery" }`.
//!
//! The metrics views (E13-S08, S09, S10) draw with the same components; the page is where a
//! reviewer or a theme author sees them together. It reads no cluster, so the data are the
//! deterministic fixtures of `oxikube_ui::chart::gallery`, ending at the current time.

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Window, div,
};
use oxikube_ui::IconName;
use oxikube_ui::chart::gallery::ChartGallery;
use oxikube_workspace::{Item, ItemEvent, TabContent};

/// The id of the gallery in `view::Open`, and its tab key.
pub const CHART_GALLERY_VIEW: &str = "chart-gallery";

/// The gallery as a tab.
pub struct ChartGalleryPage {
    gallery: Entity<ChartGallery>,
    focus: FocusHandle,
}

impl ChartGalleryPage {
    /// A gallery whose data end at `now` (seconds since the Unix epoch).
    pub fn new(now: f64, cx: &mut Context<Self>) -> Self {
        Self {
            gallery: cx.new(|cx| ChartGallery::new(now, cx)),
            focus: cx.focus_handle(),
        }
    }
}

impl EventEmitter<ItemEvent> for ChartGalleryPage {}

impl Focusable for ChartGalleryPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ChartGalleryPage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.gallery.clone())
    }
}

impl Item for ChartGalleryPage {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new("Chart gallery").icon(IconName::ChartLine)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(CHART_GALLERY_VIEW.into())
    }
}
