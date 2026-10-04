//! `oxikube_assets` — layer: `platform`.
//!
//! Embedded assets: Lucide icons, fonts, bundled themes, default settings/keymaps.
//!
//! Module map:
//! - [`icons`]: [`IconName`], the closed enum of Lucide SVGs Oxikube ships. Only the icons listed
//!   there are embedded in the binary, so an unused icon costs nothing.
//! - [`source`]: [`Assets`], the `gpui::AssetSource` that serves those icons by path
//!   (`icons/<name>.svg`).
//!
//! Fonts, bundled themes and default settings/keymaps join this crate with their own stories
//! (E05-S06 to E05-S08). `oxikube_ui` composes [`Assets`] with the component library's own icon
//! bundle; applications register the composite when they build the GPUI `Application`.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod icons;
pub mod source;

pub use icons::IconName;
pub use source::Assets;
