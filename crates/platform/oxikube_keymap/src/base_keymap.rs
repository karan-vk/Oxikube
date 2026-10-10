//! The `base_keymap` setting: which optional layer sits between the per-OS defaults and the
//! user's `keymap.json` (E11-S09).
//!
//! `"base_keymap": "vim"` in `settings.json` turns the embedded `vim.json` layer on, `"default"`
//! (the default) turns it off. The value is read once at [`init`](crate::init) and followed from
//! then on: editing `settings.json` swaps the layer and rebinds without a restart, and a reload
//! that leaves the value equal rebinds nothing (the settings observer only fires on a change).

use gpui::App;
use oxikube_settings::Settings;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::global::set_vim_layer;
use crate::store::KeymapStore;

/// The base keymap a user chooses: the layer under their own `keymap.json`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BaseKeymap {
    /// The per-OS defaults only.
    #[default]
    Default,
    /// The defaults plus the vim layer (`vim.json`): `j` / `k` / `g g` / `shift-g` / `ctrl-d` /
    /// `ctrl-u` in tables, `d d` to delete, `y y` to copy the name. Nothing changes in text
    /// fields, the terminal or the manifest editor.
    Vim,
}

impl BaseKeymap {
    /// Whether this base keymap switches the vim layer on.
    pub const fn is_vim(self) -> bool {
        matches!(self, Self::Vim)
    }
}

/// What one settings layer says about the base keymap.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct KeymapSettingsContent {
    /// The keymap layered under your `keymap.json`: `default` (the per-OS defaults) or `vim`
    /// (adds `j` / `k` / `g g` / `shift-g` / `ctrl-d` / `ctrl-u`, `d d` delete and `y y` copy name
    /// to resource tables; the editor, the terminal and text fields are untouched).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_keymap: Option<BaseKeymap>,
}

/// The resolved keymap settings: root-level keys of `settings.json`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeymapSettings {
    /// The chosen base keymap.
    pub base_keymap: BaseKeymap,
}

impl Settings for KeymapSettings {
    const KEY: Option<&'static str> = None;
    type Content = KeymapSettingsContent;

    fn from_content(content: KeymapSettingsContent) -> Self {
        Self {
            base_keymap: content.base_keymap.unwrap_or_default(),
        }
    }
}

oxikube_settings::register_settings!(KeymapSettings);

/// Let the `base_keymap` setting decide the vim layer of `store`, which is about to be installed
/// (so the first merge already has the chosen layer). Without a settings store, or without this
/// setting registered in it, the store keeps the options it was built with.
pub(crate) fn apply_to_new_store(cx: &App, store: &mut KeymapStore) {
    if let Some(settings) = KeymapSettings::try_get(cx) {
        store.set_vim(settings.base_keymap.is_vim());
    }
}

/// Follow later changes of the `base_keymap` setting: swap the layer and rebind. Reloads that
/// leave the value equal never get here (the settings observer fires on a real change only).
pub(crate) fn follow_base_keymap(cx: &mut App) {
    if KeymapSettings::try_get(cx).is_none() {
        return;
    }
    KeymapSettings::observe(cx, |cx| {
        let Some(settings) = KeymapSettings::try_get(cx) else {
            return;
        };
        let wanted = settings.base_keymap.is_vim();
        if cx
            .try_global::<KeymapStore>()
            .is_some_and(|store| store.vim_enabled() != wanted)
        {
            tracing::info!(base_keymap = ?settings.base_keymap, "base keymap changed");
            set_vim_layer(cx, wanted);
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_the_default_keymap() {
        assert_eq!(
            KeymapSettings::from_content(KeymapSettingsContent::default()).base_keymap,
            BaseKeymap::Default
        );
        assert!(!BaseKeymap::default().is_vim());
    }

    #[test]
    fn names_are_snake_case_and_unknown_ones_are_rejected() {
        let vim: KeymapSettingsContent = serde_json::from_str(r#"{"base_keymap":"vim"}"#).unwrap();
        assert_eq!(vim.base_keymap, Some(BaseKeymap::Vim));
        let default: KeymapSettingsContent =
            serde_json::from_str(r#"{"base_keymap":"default"}"#).unwrap();
        assert_eq!(default.base_keymap, Some(BaseKeymap::Default));
        assert!(
            serde_json::from_str::<KeymapSettingsContent>(r#"{"base_keymap":"emacs"}"#).is_err()
        );
    }
}
