//! Icons: [`IconName`] (our Lucide enum) and the [`Icon`] element that draws one.
//!
//! The active icon theme (`oxikube_theme::icons`, E22-S11) decides which SVG an icon is drawn
//! from: [`Icon::new`] asks it for the path of an [`IconName`], [`Icon::kind`] for a Kubernetes
//! kind's icon and [`Icon::status`] for a status's. The answer comes from a table built once per
//! theme change, so building an icon in `render` is a lookup and a reference-count bump.

use gpui::{App, Hsla, IntoElement, Pixels, RenderOnce, SharedString, Window};
use gpui_component::Sizable as _;
use gpui_component::Size as LibSize;
use oxikube_theme::icons::{icon_path, kind_icon_path, status_icon_path};

pub use oxikube_assets::IconName;

/// An icon. Takes an [`IconName`], a kind or a status, never a path: the active icon theme
/// supplies the path (Lucide when it has nothing for it).
///
/// Size and colour default to the surrounding text style, so an icon next to a label matches it.
/// Pass pixel sizes through [`crate::u`] so they follow UI zoom.
///
/// ```ignore
/// Icon::new(IconName::Box).size(u(px(14.))).color(cx.colors().text_muted)
/// ```
#[derive(IntoElement)]
pub struct Icon {
    path: SharedString,
    size: Option<Pixels>,
    color: Option<Hsla>,
}

impl Icon {
    /// An icon drawn with the surrounding text size and colour.
    pub fn new(name: IconName) -> Self {
        Self::from_path(icon_path(name))
    }

    /// The icon of `kind` in API `group` (empty for the core group): the icon theme's
    /// `kind:<group>/<Kind>` or `kind:<Kind>`, else the kind's Lucide icon.
    pub fn kind(group: &str, kind: &str) -> Self {
        Self::from_path(kind_icon_path(group, kind))
    }

    /// The icon of a status text (`Running`, `CrashLoopBackOff`): the icon theme's
    /// `status:<Status>`, else a Lucide icon for what the status means.
    pub fn status(status: &str) -> Self {
        Self::from_path(status_icon_path(status))
    }

    fn from_path(path: SharedString) -> Self {
        Self {
            path,
            size: None,
            color: None,
        }
    }

    /// The asset path this icon draws from, as the active icon theme resolved it.
    pub fn path(&self) -> &SharedString {
        &self.path
    }

    /// Square edge length.
    #[must_use]
    pub fn size(mut self, size: Pixels) -> Self {
        self.size = Some(size);
        self
    }

    /// Stroke colour.
    #[must_use]
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
        let mut icon = gpui_component::Icon::default().path(self.path);
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
