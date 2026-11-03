//! [`SourceLayers`]: named items kept per [`ThemeSource`], with the registry's precedence (user,
//! then extensions in id order, then bundled). The icon-theme half of the registry is built on
//! it.

use super::{ExtensionId, ThemeSource};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Items by name, one map per source.
#[derive(Clone, Debug)]
pub(crate) struct SourceLayers<T> {
    bundled: BTreeMap<String, Arc<T>>,
    extensions: BTreeMap<ExtensionId, BTreeMap<String, Arc<T>>>,
    user: BTreeMap<String, Arc<T>>,
}

impl<T> Default for SourceLayers<T> {
    fn default() -> Self {
        Self {
            bundled: BTreeMap::new(),
            extensions: BTreeMap::new(),
            user: BTreeMap::new(),
        }
    }
}

impl<T> SourceLayers<T> {
    /// The map of `source`, created for a new extension.
    pub(crate) fn layer_mut(&mut self, source: &ThemeSource) -> &mut BTreeMap<String, Arc<T>> {
        match source {
            ThemeSource::Bundled => &mut self.bundled,
            ThemeSource::User => &mut self.user,
            ThemeSource::Extension(id) => self.extensions.entry(id.clone()).or_default(),
        }
    }

    /// Removes everything `source` holds; returns how many there were.
    pub(crate) fn remove(&mut self, source: &ThemeSource) -> usize {
        match source {
            ThemeSource::Bundled => std::mem::take(&mut self.bundled).len(),
            ThemeSource::User => std::mem::take(&mut self.user).len(),
            ThemeSource::Extension(id) => self.extensions.remove(id).map_or(0, |layer| layer.len()),
        }
    }

    /// The item called `name` and the source that wins for it.
    pub(crate) fn lookup(&self, name: &str) -> Option<(&Arc<T>, ThemeSource)> {
        if let Some(item) = self.user.get(name) {
            return Some((item, ThemeSource::User));
        }
        let from_extension = self
            .extensions
            .iter()
            .find_map(|(id, layer)| layer.get(name).map(|item| (item, id)));
        if let Some((item, id)) = from_extension {
            return Some((item, ThemeSource::Extension(id.clone())));
        }
        self.bundled
            .get(name)
            .map(|item| (item, ThemeSource::Bundled))
    }

    /// Every name any source has, once.
    pub(crate) fn names(&self) -> BTreeSet<&str> {
        self.user
            .keys()
            .chain(self.extensions.values().flat_map(BTreeMap::keys))
            .chain(self.bundled.keys())
            .map(String::as_str)
            .collect()
    }

    /// The bundled item called `name`.
    pub(crate) fn bundled(&self, name: &str) -> Option<&Arc<T>> {
        self.bundled.get(name)
    }
}
