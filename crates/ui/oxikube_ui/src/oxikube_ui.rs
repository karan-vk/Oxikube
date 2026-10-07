//! `oxikube_ui` — layer: `ui`.
//!
//! Thin wrapper over gpui-component: tokens, curated components (Table, DockArea, Dialog, Menu,
//! Input, Tabs, Sidebar, Charts, Markdown, Editor glue), icons, zoom-safe sizes. The ONLY crate
//! allowed to import gpui_component (`cargo xtask lint-deps` enforces it): every other crate takes
//! what it needs from here, under our names, so a gpui-component bump or replacement touches one
//! crate (ADR 0004).
//!
//! Module map:
//! - [`setup`]: [`init`] (one call to initialise and theme the library), [`set_token_source`].
//! - [`tokens`]: [`Tokens`] (colours, spacing, radius, font sizes), [`ActiveTokens`]
//!   (`cx.tokens()` / `cx.colors()`), the [`TokenSource`] adapter trait.
//! - [`theme_bridge`]: projects tokens and `oxikube_theme` themes (`set_theme`, `theme_config`,
//!   `follow_active_theme`) onto gpui-component's theme (the only writer of it).
//! - [`size`]: [`u`] zoom-safe sizes, [`UiScale`], [`Unscaled`] for persisted sizes.
//! - [`motion`]: [`motion::reduce_motion`], what animations check (E05-S12 resolves it).
//! - [`icon`]: [`IconName`] (Lucide, embedded by `oxikube_assets`) and the [`Icon`] element.
//! - [`assets`]: [`Assets`], the application asset source to pass to `Application::with_assets`.
//! - [`table`]: [`Table`] over our own [`TableDelegate`] trait (virtualised, uniform rows).
//! - [`editor`]: the read-only, tree-sitter highlighted code view the YAML tab shows.
//! - [`dock`], [`dialog`], [`menu`], [`input`], [`tabs`], [`sidebar`], [`chart`], [`markdown`],
//!   [`button`], [`layout`]: curated re-exports under our names; no `pub use gpui_component::*`.
//! - [`error_details`]: the Details toggle and raw-text box every error notice shares.
//! - [`spinner`]: [`spinner::Spinner`], a loading indicator that stands still under reduce-motion.
//! - [`tooltip`]: [`tooltip::Tooltip`], hover text for any element.
//! - [`tile`]: [`tile::StatTile`], a clickable number with a caption (overview pages).
//! - [`root`]: the window root, which owns the dialog, sheet and notification layers.
//! - [`title_bar`]: the window title bar (drag area, window controls, traffic-light inset).
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction.

pub mod assets;
pub mod button;
pub mod chart;
pub mod dialog;
pub mod dock;
pub mod editor;
pub mod error_details;
pub mod icon;
pub mod input;
pub mod layout;
pub mod markdown;
pub mod menu;
pub mod motion;
pub mod root;
pub mod setup;
pub mod sidebar;
pub mod size;
pub mod spinner;
pub mod table;
pub mod tabs;
pub mod theme_bridge;
pub mod tile;
pub mod title_bar;
pub mod tokens;
pub mod tooltip;

pub use assets::Assets;
pub use icon::{Icon, IconName};
pub use setup::{init, set_token_source};
pub use size::{ControlSize, Sizable, UiScale, Unscaled, set_ui_scale, u};
pub use table::{Table, TableColumn, TableDelegate, TableHandle};
pub use theme_bridge::{follow_active_theme, set_theme, set_tokens, theme_config};
pub use tokens::{
    ActiveTokens, Appearance, Colors, FontSizes, Radius, Spacing, TokenSource, Tokens,
};
