//! `ThemeRegistry`: every theme the app can show, by name.
//!
//! Pure data (no GPUI), so selection logic is unit-testable with a fake system appearance.
//! Bundled themes (One Dark, One Light) are always present; user themes come from the
//! `themes/` directory (`user_dir`) and replace each other wholesale on every rescan, so a
//! deleted or renamed file disappears from [`ThemeRegistry::list`]. A user theme with the same
//! name as a bundled one wins.

use crate::appearance::Appearance;
use crate::import::{ThemeFamily, import_family};
use crate::settings::ThemeSelection;
use crate::tokens::ThemeTokens;
use crate::user_dir::UserThemes;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Where a theme came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeSource {
    /// Embedded in the application.
    Bundled,
    /// A file in the user's `themes/` directory.
    User,
}

/// A theme as listed in a picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeMeta {
    /// The theme's name.
    pub name: String,
    /// Light or dark.
    pub appearance: Appearance,
    /// Bundled or user-provided.
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

    /// Adds a family to the bundled set (also how extensions will contribute themes).
    pub fn insert_bundled(&mut self, family: ThemeFamily) {
        for theme in family.themes {
            self.bundled.insert(theme.name.clone(), Arc::new(theme));
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

    /// The theme called `name` (a user theme before a bundled one of the same name).
    pub fn get(&self, name: &str) -> Option<Arc<ThemeTokens>> {
        self.user
            .get(name)
            .or_else(|| self.bundled.get(name))
            .cloned()
    }

    /// Every theme once, sorted by name (case-insensitive), user themes shadowing bundled ones.
    pub fn list(&self) -> Vec<ThemeMeta> {
        let mut metas: Vec<ThemeMeta> = self
            .user
            .values()
            .map(|theme| (theme, ThemeSource::User))
            .chain(
                self.bundled
                    .values()
                    .filter(|theme| !self.user.contains_key(&theme.name))
                    .map(|theme| (theme, ThemeSource::Bundled)),
            )
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
