//! Choosing a theme: which `theme` setting slot an edit goes to, and writing it.
//!
//! The picker previews with [`crate::preview_theme`] and, on confirm, calls [`commit_theme`]. The
//! write goes through the settings store's comment-preserving update, so a user's `settings.json`
//! keeps its comments and formatting.
//!
//! # Which slot
//!
//! The `theme` setting is either one name (static) or `{ mode, light, dark }`. A pick edits the
//! slot that is in effect right now ([`edited_slot`]): the whole value when it is static, the
//! `light` or `dark` entry the mode and the system appearance select otherwise. So the theme the
//! user just looked at is the one they keep, even when it is dark and the system is light; the
//! other slot and `mode` are left alone.

use crate::appearance::{Appearance, SystemAppearance, ThemeMode};
use crate::preview::clear_preview;
use crate::settings::{ThemeSelection, ThemeSelectionContent, ThemeSettings};
use gpui::{App, Task};
use oxikube_domain::OxiResult;
use oxikube_settings::Settings as _;

/// Which part of the `theme` setting a pick edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeSlot {
    /// The setting is one theme name: a pick replaces it.
    Any,
    /// The `light` entry (mode light, or system while the OS is light).
    Light,
    /// The `dark` entry (mode dark, or system while the OS is dark).
    Dark,
}

impl ThemeSlot {
    /// The slot's name for a sentence: "theme", "light theme" or "dark theme".
    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "theme",
            Self::Light => "light theme",
            Self::Dark => "dark theme",
        }
    }
}

/// The slot a pick edits under `selection` with the system in `system` appearance.
pub fn edited_slot(selection: &ThemeSelection, system: Appearance) -> ThemeSlot {
    match selection {
        ThemeSelection::Static(_) => ThemeSlot::Any,
        ThemeSelection::Dynamic { .. } => match selection.appearance_for(system) {
            Appearance::Light => ThemeSlot::Light,
            Appearance::Dark => ThemeSlot::Dark,
        },
    }
}

/// What the `theme` setting says right now, for the picker's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeChoice {
    /// The slot a pick edits.
    pub slot: ThemeSlot,
    /// The mode of a `{ mode, light, dark }` setting; `None` for a plain theme name.
    pub mode: Option<ThemeMode>,
}

/// The slot a pick edits now and the mode behind it, from the settings and the system appearance
/// (the defaults without a settings store).
pub fn current_choice(cx: &App) -> ThemeChoice {
    let selection = ThemeSettings::try_get(cx)
        .map(|settings| settings.selection.clone())
        .unwrap_or_default();
    ThemeChoice {
        slot: edited_slot(&selection, SystemAppearance::get(cx)),
        mode: match selection {
            ThemeSelection::Static(_) => None,
            ThemeSelection::Dynamic { mode, .. } => Some(mode),
        },
    }
}

/// Sets `name` in the slot of `content` that is in effect (see the module docs). A `content` with
/// no `theme` key yet becomes `{ "light": .. }` or `{ "dark": .. }`, keeping the default mode.
pub fn set_theme_name(content: &mut ThemeSelectionContent, name: &str, system: Appearance) {
    match content {
        ThemeSelectionContent::Static(current) => name.clone_into(current),
        ThemeSelectionContent::Dynamic { mode, light, dark } => {
            let slot = match mode.unwrap_or_default().appearance(system) {
                Appearance::Light => light,
                Appearance::Dark => dark,
            };
            *slot = Some(name.to_owned());
        }
    }
}

/// Writes `name` into the user's `theme` setting (see the module docs) and, once the store has
/// applied it, ends the preview so the committed theme takes over without a flash of the old one.
///
/// The preview stays in effect until the write lands, whatever the outcome: on failure the
/// preview is ended too and the error comes back in the task. Needs [`crate::init`] and the
/// settings store.
pub fn commit_theme(cx: &mut App, name: &str) -> Task<OxiResult<()>> {
    let system = SystemAppearance::get(cx);
    let name = name.to_owned();
    let write = oxikube_settings::update_user_settings::<ThemeSettings>(cx, None, move |content| {
        set_theme_name(content, &name, system);
    });
    cx.spawn(async move |cx| {
        let result = write.await;
        cx.update(clear_preview);
        result
    })
}

#[cfg(test)]
mod tests;
