//! The design-token value types.
//!
//! Plain data: no GPUI context, no gpui-component types. [`crate::theme_bridge`] projects a
//! [`Tokens`] onto the component library; views read it through [`crate::ActiveTokens`].

use gpui::{Hsla, Pixels, WindowAppearance, px};

/// Light or dark. Mirrors the two halves of a theme family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Appearance {
    /// Light surfaces, dark text.
    Light,
    /// Dark surfaces, light text.
    #[default]
    Dark,
}

impl Appearance {
    /// Whether this is [`Appearance::Dark`].
    pub fn is_dark(self) -> bool {
        self == Appearance::Dark
    }
}

impl From<WindowAppearance> for Appearance {
    fn from(value: WindowAppearance) -> Self {
        match value {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Appearance::Light,
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Appearance::Dark,
        }
    }
}

/// Semantic colours. Names follow Zed's theme vocabulary so the E05-S08 importer maps 1:1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Colors {
    /// Window and editor background.
    pub background: Hsla,
    /// Panels, docks and the status bar.
    pub surface: Hsla,
    /// Popovers, menus and dialogs: one step above [`Colors::surface`].
    pub elevated_surface: Hsla,
    /// Primary text.
    pub text: Hsla,
    /// Secondary text (captions, column headers).
    pub text_muted: Hsla,
    /// Disabled text and placeholders.
    pub text_disabled: Hsla,
    /// Strong borders (inputs, dividers between docks).
    pub border: Hsla,
    /// Quiet borders (table row separators).
    pub border_variant: Hsla,
    /// Border of the focused control.
    pub border_focused: Hsla,
    /// Resting background of an interactive element (button, input).
    pub element: Hsla,
    /// Element under the pointer; also table row hover.
    pub element_hover: Hsla,
    /// Element being pressed.
    pub element_active: Hsla,
    /// Selected element (table row, active tab).
    pub element_selected: Hsla,
    /// Accent: primary buttons, links, focus ring.
    pub accent: Hsla,
    /// Text drawn on top of [`Colors::accent`].
    pub on_accent: Hsla,
    /// Positive state (Running, Ready).
    pub success: Hsla,
    /// Needs attention (Pending, Warning events).
    pub warning: Hsla,
    /// Failure (CrashLoopBackOff, Error).
    pub error: Hsla,
    /// Neutral information.
    pub info: Hsla,
    /// Text selection background.
    pub selection: Hsla,
}

/// Spacing scale in unscaled pixels. Wrap with [`crate::u`] when laying out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spacing {
    /// 2 px: icon-to-label gaps.
    pub xs: Pixels,
    /// 4 px: tight padding.
    pub sm: Pixels,
    /// 8 px: default gap and padding.
    pub md: Pixels,
    /// 12 px: section padding.
    pub lg: Pixels,
    /// 16 px: panel padding.
    pub xl: Pixels,
    /// 24 px: dialog padding.
    pub xxl: Pixels,
}

/// Corner radii in unscaled pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Radius {
    /// Small chips and table cells.
    pub sm: Pixels,
    /// Buttons, inputs, general elements.
    pub md: Pixels,
    /// Dialogs, popovers, notifications.
    pub lg: Pixels,
}

/// Font sizes in unscaled pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontSizes {
    /// Captions and dense table cells.
    pub small: Pixels,
    /// Default UI text.
    pub body: Pixels,
    /// Section headings and dialog titles.
    pub heading: Pixels,
    /// Monospace text (logs, YAML).
    pub mono: Pixels,
}

/// Everything a view needs to look right: colours, spacing, radii and font sizes.
///
/// Sizes are stored **unscaled**; multiply with [`crate::u`] at the point of use so UI zoom keeps
/// working.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    /// Which half of the theme these tokens describe.
    pub appearance: Appearance,
    /// Semantic colours.
    pub colors: Colors,
    /// Spacing scale.
    pub spacing: Spacing,
    /// Corner radii.
    pub radius: Radius,
    /// Font sizes.
    pub font: FontSizes,
}

impl Tokens {
    /// The built-in tokens for `appearance`: used until `oxikube_theme` provides a theme.
    pub fn default_for(appearance: Appearance) -> Self {
        match appearance {
            Appearance::Dark => Self::dark(),
            Appearance::Light => Self::light(),
        }
    }

    /// Built-in dark tokens (One-Dark-like). Text and status colours are at least 4.5:1 (WCAG AA)
    /// on every background they are drawn on; `tokens::contrast` tests it.
    pub fn dark() -> Self {
        Self {
            appearance: Appearance::Dark,
            colors: Colors {
                background: hex(0x1e2127),
                surface: hex(0x21252b),
                elevated_surface: hex(0x2b3039),
                text: hex(0xdce0e5),
                text_muted: hex(0xb6bbc6),
                text_disabled: hex(0x6b727f),
                border: hex(0x363c46),
                border_variant: hex(0x2c313a),
                border_focused: hex(0x47679e),
                element: hex(0x2e343e),
                element_hover: hex(0x363c46),
                element_active: hex(0x454a56),
                element_selected: hex(0x2a4268),
                accent: hex(0x7fb4ea),
                on_accent: hex(0x0f1114),
                success: hex(0xa1c181),
                warning: hex(0xdec184),
                error: hex(0xdf9fa3),
                info: hex(0x7fb4ea),
                selection: hex_alpha(0x74ade8, 0.3),
            },
            spacing: Spacing::default(),
            radius: Radius::default(),
            font: FontSizes::default(),
        }
    }

    /// Built-in light tokens. Text and status colours meet WCAG AA like [`Tokens::dark`]'s.
    pub fn light() -> Self {
        Self {
            appearance: Appearance::Light,
            colors: Colors {
                background: hex(0xfafafa),
                surface: hex(0xebebec),
                elevated_surface: hex(0xffffff),
                text: hex(0x2a2c33),
                text_muted: hex(0x58585a),
                text_disabled: hex(0xa1a1a3),
                border: hex(0xc9c9cc),
                border_variant: hex(0xdcdcdd),
                border_focused: hex(0x7a9bd9),
                element: hex(0xe6e6e8),
                element_hover: hex(0xdcdcde),
                element_active: hex(0xcdcdd1),
                element_selected: hex(0xd1defa),
                accent: hex(0x2a5db3),
                on_accent: hex(0xffffff),
                success: hex(0x3d6a2f),
                warning: hex(0x7c590d),
                error: hex(0xaa373f),
                info: hex(0x2a5db3),
                selection: hex_alpha(0x3b73d1, 0.25),
            },
            spacing: Spacing::default(),
            radius: Radius::default(),
            font: FontSizes::default(),
        }
    }
}

impl Default for Tokens {
    fn default() -> Self {
        Self::dark()
    }
}

impl Default for Spacing {
    fn default() -> Self {
        Self {
            xs: px(2.),
            sm: px(4.),
            md: px(8.),
            lg: px(12.),
            xl: px(16.),
            xxl: px(24.),
        }
    }
}

impl Default for Radius {
    fn default() -> Self {
        Self {
            sm: px(3.),
            md: px(6.),
            lg: px(10.),
        }
    }
}

impl Default for FontSizes {
    fn default() -> Self {
        Self {
            small: px(12.),
            body: px(14.),
            heading: px(16.),
            mono: px(13.),
        }
    }
}

/// `0xRRGGBB` to [`Hsla`].
pub(crate) fn hex(rgb: u32) -> Hsla {
    gpui::rgb(rgb).into()
}

/// `0xRRGGBB` with an alpha in `0.0..=1.0`.
pub(crate) fn hex_alpha(rgb: u32, alpha: f32) -> Hsla {
    hex(rgb).opacity(alpha)
}
