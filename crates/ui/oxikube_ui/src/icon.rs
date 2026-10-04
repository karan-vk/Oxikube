//! Icons: [`IconName`] (our Lucide enum) and the [`Icon`] element that draws one.

use gpui::{App, Hsla, IntoElement, Pixels, RenderOnce, Window};
use gpui_component::Sizable as _;
use gpui_component::Size as LibSize;

pub use oxikube_assets::IconName;

/// A Lucide icon. Takes an [`IconName`], never a path.
///
/// Size and colour default to the surrounding text style, so an icon next to a label matches it.
/// Pass pixel sizes through [`crate::u`] so they follow UI zoom.
///
/// ```ignore
/// Icon::new(IconName::Box).size(u(px(14.))).color(cx.colors().text_muted)
/// ```
#[derive(IntoElement)]
pub struct Icon {
    name: IconName,
    size: Option<Pixels>,
    color: Option<Hsla>,
}

impl Icon {
    /// An icon drawn with the surrounding text size and colour.
    pub fn new(name: IconName) -> Self {
        Self {
            name,
            size: None,
            color: None,
        }
    }

    /// Square edge length.
    pub fn size(mut self, size: Pixels) -> Self {
        self.size = Some(size);
        self
    }

    /// Stroke colour.
    pub fn color(mut self, color: impl Into<Hsla>) -> Self {
        self.color = Some(color.into());
        self
    }
}

impl From<IconName> for Icon {
    fn from(name: IconName) -> Self {
        Icon::new(name)
    }
}

impl Icon {
    fn into_library(self) -> gpui_component::Icon {
        let mut icon = gpui_component::Icon::default().path(self.name.path());
        if let Some(size) = self.size {
            icon = icon.with_size(LibSize::Size(size));
        }
        if let Some(color) = self.color {
            icon = gpui::Styled::text_color(icon, color);
        }
        icon
    }
}

/// Lets an [`Icon`] go wherever a component takes an icon (tab, sidebar item, button).
impl From<Icon> for gpui_component::Icon {
    fn from(icon: Icon) -> Self {
        icon.into_library()
    }
}

impl RenderOnce for Icon {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        self.into_library()
    }
}
