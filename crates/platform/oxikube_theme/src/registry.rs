//! `ThemeRegistry`: every theme the app can show, by name.
//!
//! Pure data (no GPUI), so selection logic is unit-testable with a fake system appearance.
//! Bundled themes (One Dark, One Light) are always present; user themes come from the
//! `themes/` directory (`user_dir`) and replace each other wholesale on every rescan, so a
//! deleted or renamed file disappears from [`ThemeRegistry::list`]. Extensions contribute
//! families under their own [`ExtensionId`] and can be removed again without touching the other
//! sources.
//!
//! Duplicate names: within one source the later insert wins; across sources the order is
//! user, then extension (lowest [`ExtensionId`] first), then bundled.

use crate::appearance::Appearance;
use crate::import::{ThemeFamily, import_family};
use crate::settings::ThemeSelection;
use crate::tokens::ThemeTokens;
use crate::user_dir::UserThemes;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

/// The identity of an extension that contributes themes.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExtensionId(Arc<str>);

impl ExtensionId {
    /// An id from its manifest string.
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self(id.into())
    }

    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Where a theme came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeSource {
    /// Embedded in the application.
    Bundled,
    /// A file in the user's `themes/` directory.
    User,
    /// Contributed by an extension.
    Extension(ExtensionId),
}

/// A theme as listed in a picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeMeta {
    /// The theme's name.
    pub name: String,
    /// Light or dark.
    pub appearance: Appearance,
    /// Bundled, user-provided or from an extension.
    pub source: ThemeSource,
}

/// A file in the themes directory that could not be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeFileProblem {
    /// The file.
    pub path: PathBuf,
    /// What is wrong with it (first problem for a partly valid file).
    pub message: String,
}

/// All known themes.
#[derive(Clone, Debug, Default)]
pub struct ThemeRegistry {
    bundled: BTreeMap<String, Arc<ThemeTokens>>,
    extensions: BTreeMap<ExtensionId, BTreeMap<String, Arc<ThemeTokens>>>,
    user: BTreeMap<String, Arc<ThemeTokens>>,
    problems: Vec<ThemeFileProblem>,
}

/// The name of the bundled dark theme: the default for a dark appearance.
pub const DEFAULT_DARK_THEME: &str = "One Dark";
/// The name of the bundled light theme: the default for a light appearance.
pub const DEFAULT_LIGHT_THEME: &str = "One Light";

impl ThemeRegistry {
    /// A registry holding the bundled themes (parses the embedded files: well under a
    /// millisecond).
    pub fn with_bundled() -> Self {
        let mut registry = Self::default();
        for file in oxikube_assets::BUNDLED_THEME_FAMILIES {
            match import_family(file.json) {
                Ok(imported) => registry.insert_bundled(imported.family),
                Err(err) => tracing::error!(file = file.file, %err, "bundled theme is invalid"),
            }
        }
        registry
    }

    /// Adds a family to the bundled set.
    pub fn insert_bundled(&mut self, family: ThemeFamily) {
        self.insert_theme_families(&ThemeSource::Bundled, [family]);
    }

    /// Adds `families` under `source`; a theme whose name the source already has is replaced.
    pub fn insert_theme_families(
        &mut self,
        source: &ThemeSource,
        families: impl IntoIterator<Item = ThemeFamily>,
    ) {
        let layer = match source {
            ThemeSource::Bundled => &mut self.bundled,
            ThemeSource::User => &mut self.user,
            ThemeSource::Extension(id) => self.extensions.entry(id.clone()).or_default(),
        };
        for theme in families.into_iter().flat_map(|family| family.themes) {
            layer.insert(theme.name.clone(), Arc::new(theme));
        }
    }

    /// Removes every theme `source` contributed and returns how many there were; other
    /// sources are untouched. Removing an unknown extension is a no-op.
    pub fn remove_by_source(&mut self, source: &ThemeSource) -> usize {
        match source {
            ThemeSource::Bundled => std::mem::take(&mut self.bundled).len(),
            ThemeSource::User => std::mem::take(&mut self.user).len(),
            ThemeSource::Extension(id) => self.extensions.remove(id).map_or(0, |layer| layer.len()),
        }
    }

    /// Replaces every user theme with the contents of a fresh scan of the themes directory.
    pub fn replace_user_themes(&mut self, scan: UserThemes) {
        self.user.clear();
        self.problems = scan.problems;
        for (path, family) in scan.families {
            for theme in family.themes {
                if self.user.contains_key(&theme.name) {
                    self.problems.push(ThemeFileProblem {
                        path: path.clone(),
                        message: format!("duplicate theme name {:?}; first one kept", theme.name),
                    });
                    continue;
                }
                self.user.insert(theme.name.clone(), Arc::new(theme));
            }
        }
    }

    /// The theme called `name` and the source that wins for it.
    fn lookup(&self, name: &str) -> Option<(&Arc<ThemeTokens>, ThemeSource)> {
        if let Some(theme) = self.user.get(name) {
            return Some((theme, ThemeSource::User));
        }
        let from_extension = self
            .extensions
            .iter()
            .find_map(|(id, layer)| layer.get(name).map(|theme| (theme, id)));
        if let Some((theme, id)) = from_extension {
            return Some((theme, ThemeSource::Extension(id.clone())));
        }
        self.bundled
            .get(name)
            .map(|theme| (theme, ThemeSource::Bundled))
    }

    /// The theme called `name` (see the module docs for which source wins a duplicate).
    pub fn get(&self, name: &str) -> Option<Arc<ThemeTokens>> {
        self.lookup(name).map(|(theme, _)| theme.clone())
    }

    /// Every theme once, sorted by name (case-insensitive), higher-precedence sources
    /// shadowing lower ones.
    pub fn list(&self) -> Vec<ThemeMeta> {
        let names: BTreeSet<&str> = self
            .user
            .keys()
            .chain(self.extensions.values().flat_map(BTreeMap::keys))
            .chain(self.bundled.keys())
            .map(String::as_str)
            .collect();
        let mut metas: Vec<ThemeMeta> = names
            .into_iter()
            .filter_map(|name| self.lookup(name))
            .map(|(theme, source)| ThemeMeta {
                name: theme.name.clone(),
                appearance: theme.appearance,
                source,
            })
            .collect();
        metas.sort_by_key(|meta| meta.name.to_lowercase());
        metas
    }

    /// Theme names only, in [`ThemeRegistry::list`] order.
    pub fn names(&self) -> Vec<String> {
        self.list().into_iter().map(|meta| meta.name).collect()
    }

    /// Files of the themes directory that failed to load, from the last scan.
    pub fn problems(&self) -> &[ThemeFileProblem] {
        &self.problems
    }

    /// The default theme for `appearance` (One Dark / One Light).
    pub fn default_for(&self, appearance: Appearance) -> Arc<ThemeTokens> {
        let name = if appearance.is_dark() {
            DEFAULT_DARK_THEME
        } else {
            DEFAULT_LIGHT_THEME
        };
        self.bundled
            .get(name)
            .cloned()
            .unwrap_or_else(|| Arc::new(ThemeTokens::fallback(appearance).clone()))
    }

    /// The theme `selection` asks for given the system appearance. A name that is not
    /// (or not yet) registered resolves to the default theme of the appearance the selection
    /// wants, so a typo or a still-loading user theme never leaves the app unthemed.
    pub fn resolve(&self, selection: &ThemeSelection, system: Appearance) -> Arc<ThemeTokens> {
        self.get(selection.name_for(system))
            .unwrap_or_else(|| self.default_for(selection.appearance_for(system)))
    }
}

#[cfg(test)]
mod tests;
