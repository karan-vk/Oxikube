//! The icon-theme half of [`ThemeRegistry`] (E22-S11), symmetrical with the colour themes:
//! bundled (Lucide), the user's `icon_themes/` directory, and extensions under their own
//! [`ThemeSource::Extension`] (loaded by E23-S06 through the same insert and remove calls).
//!
//! Pure data like the rest of the registry: reading icon files is
//! [`read_icon_theme_file`](crate::icons::read_icon_theme_file)'s job, off the UI thread.

use super::layers::SourceLayers;
use super::{ThemeFileProblem, ThemeRegistry, ThemeSource};
use crate::appearance::Appearance;
use crate::icons::{IconTheme, IconThemeFamily, IconThemeSource};
use crate::settings::ThemeSelection;
use std::path::PathBuf;
use std::sync::Arc;

/// The icon themes of a registry.
#[derive(Clone, Debug, Default)]
pub(crate) struct IconThemeLayers {
    layers: SourceLayers<IconTheme>,
    problems: Vec<ThemeFileProblem>,
}

/// An icon theme as listed in a picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconThemeMeta {
    /// The icon theme's name.
    pub name: String,
    /// Light or dark.
    pub appearance: Appearance,
    /// Bundled, user-provided or from an extension.
    pub source: ThemeSource,
}

impl ThemeRegistry {
    /// Adds `families` under `source`; an icon theme whose name the source already has is
    /// replaced.
    pub fn insert_icon_theme_families(
        &mut self,
        source: &ThemeSource,
        families: impl IntoIterator<Item = IconThemeFamily>,
    ) {
        let layer = self.icon_themes.layers.layer_mut(source);
        for theme in families.into_iter().flat_map(|family| family.themes) {
            layer.insert(theme.name.clone(), Arc::new(theme));
        }
    }

    /// Mounts an icon-theme family read off the UI thread and adds it under `source`; returns
    /// the names of the themes it added. What an extension's loader calls per `icon_themes`
    /// entry of its manifest.
    pub fn load_icon_theme(&mut self, source: &ThemeSource, read: IconThemeSource) -> Vec<String> {
        let family = IconThemeFamily::mount(read);
        let names = family
            .themes
            .iter()
            .map(|theme| theme.name.clone())
            .collect();
        self.insert_icon_theme_families(source, [family]);
        names
    }

    /// Removes every icon theme `source` contributed and returns how many there were; other
    /// sources are untouched, and the removed themes' files stop being served once nothing holds
    /// them.
    pub fn remove_icon_themes(&mut self, source: &ThemeSource) -> usize {
        self.icon_themes.layers.remove(source)
    }

    /// Replaces every user icon theme with a fresh scan of the `icon_themes/` directory. Within
    /// the scan the first file (by name) that defines a theme name keeps it.
    pub fn replace_user_icon_themes(
        &mut self,
        families: Vec<(PathBuf, IconThemeFamily)>,
        problems: Vec<ThemeFileProblem>,
    ) {
        let layers = &mut self.icon_themes;
        layers.problems = problems;
        let user = layers.layers.layer_mut(&ThemeSource::User);
        user.clear();
        for (path, family) in families {
            for theme in family.themes {
                if user.contains_key(&theme.name) {
                    layers.problems.push(ThemeFileProblem {
                        path: path.clone(),
                        message: format!("duplicate icon theme name {:?}; first kept", theme.name),
                    });
                    continue;
                }
                user.insert(theme.name.clone(), Arc::new(theme));
            }
        }
    }

    /// The icon theme called `name` (user, then extensions, then bundled).
    pub fn get_icon_theme(&self, name: &str) -> Option<Arc<IconTheme>> {
        self.icon_themes
            .layers
            .lookup(name)
            .map(|(theme, _)| theme.clone())
    }

    /// Every icon theme once, sorted by name (case-insensitive).
    pub fn list_icon_themes(&self) -> Vec<IconThemeMeta> {
        let layers = &self.icon_themes.layers;
        let mut metas: Vec<IconThemeMeta> = layers
            .names()
            .into_iter()
            .filter_map(|name| layers.lookup(name))
            .map(|(theme, source)| IconThemeMeta {
                name: theme.name.clone(),
                appearance: theme.appearance,
                source,
            })
            .collect();
        metas.sort_by_key(|meta| meta.name.to_lowercase());
        metas
    }

    /// The built-in Lucide icon theme.
    pub fn default_icon_theme(&self) -> Arc<IconTheme> {
        self.icon_themes
            .layers
            .bundled(crate::icons::DEFAULT_ICON_THEME)
            .cloned()
            .unwrap_or_else(IconTheme::shared_lucide)
    }

    /// The icon theme `selection` asks for under the system appearance; a name that is not (or
    /// not yet) installed resolves to Lucide.
    pub fn resolve_icon_theme(
        &self,
        selection: &ThemeSelection,
        system: Appearance,
    ) -> Arc<IconTheme> {
        self.get_icon_theme(selection.name_for(system))
            .unwrap_or_else(|| self.default_icon_theme())
    }

    /// Files of the `icon_themes/` directory that failed to load (or loaded with dropped icons),
    /// from the last scan.
    pub fn icon_theme_problems(&self) -> &[ThemeFileProblem] {
        &self.icon_themes.problems
    }

    pub(super) fn insert_bundled_icon_themes(&mut self) {
        self.icon_themes
            .layers
            .layer_mut(&ThemeSource::Bundled)
            .insert(
                crate::icons::DEFAULT_ICON_THEME.to_owned(),
                IconTheme::shared_lucide(),
            );
    }
}

#[cfg(test)]
mod tests;
