//! [`SourceListStore`]: where the user's source list is kept.

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_ports::UserSource;
use parking_lot::Mutex;

/// Keeps the user's list of kubeconfig sources (the settings key `kubeconfig.sources`).
///
/// The app layer cannot see the settings store (it is a platform crate that needs GPUI), so the
/// service reaches it through this trait: `oxikube_catalog_ui::sources` implements it over
/// `SettingsStore`, and tests use [`MemorySourceList`].
///
/// `load` is a read of what is already in memory; `save` returns once the list is stored (for
/// settings: written to `settings.json` and applied), so a `load` after it sees the new list.
#[async_trait]
pub trait SourceListStore: Send + Sync {
    /// The list as stored now, in order.
    async fn load(&self) -> OxiResult<Vec<UserSource>>;

    /// Replaces the stored list.
    ///
    /// # Errors
    ///
    /// When the list cannot be stored (the settings file cannot be written); the stored list
    /// is unchanged then.
    async fn save(&self, sources: &[UserSource]) -> OxiResult<()>;
}

/// A [`SourceListStore`] held in memory: for tests, and for a run without a settings file.
#[derive(Debug, Default)]
pub struct MemorySourceList {
    sources: Mutex<Vec<UserSource>>,
}

impl MemorySourceList {
    /// A store holding `sources`.
    pub fn new(sources: impl IntoIterator<Item = UserSource>) -> Self {
        Self {
            sources: Mutex::new(sources.into_iter().collect()),
        }
    }

    /// The list as stored now.
    pub fn snapshot(&self) -> Vec<UserSource> {
        self.sources.lock().clone()
    }
}

#[async_trait]
impl SourceListStore for MemorySourceList {
    async fn load(&self) -> OxiResult<Vec<UserSource>> {
        Ok(self.snapshot())
    }

    async fn save(&self, sources: &[UserSource]) -> OxiResult<()> {
        *self.sources.lock() = sources.to_vec();
        Ok(())
    }
}
