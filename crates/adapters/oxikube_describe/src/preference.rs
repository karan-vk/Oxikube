//! The user's choice of describe backend, shared by every describer and changed at run time.

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;

/// Which implementation describes an object.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Backend {
    /// deskribe for every kind it covers, `kubectl describe` for the rest (when installed).
    #[default]
    Auto,
    /// deskribe only: a kind it does not cover is reported as unsupported.
    Native,
    /// `kubectl describe` only.
    Kubectl,
}

/// What the user configured for describe.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DescribeConfig {
    /// Which implementation to use.
    pub backend: Backend,
    /// The `kubectl` binary; `None` looks `kubectl` up on `PATH`.
    pub kubectl_path: Option<PathBuf>,
}

/// A shared, hot-swappable [`DescribeConfig`]. Cheap to clone; clones see each other's `set`.
#[derive(Debug, Clone, Default)]
pub struct DescribePreference {
    inner: Arc<RwLock<DescribeConfig>>,
}

impl DescribePreference {
    /// A preference holding `config`.
    pub fn new(config: DescribeConfig) -> Self {
        Self {
            inner: Arc::new(RwLock::new(config)),
        }
    }

    /// The current configuration.
    pub fn get(&self) -> DescribeConfig {
        self.inner.read().clone()
    }

    /// Replaces the configuration; the next `describe` of every describer uses it.
    pub fn set(&self, config: DescribeConfig) {
        *self.inner.write() = config;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_the_value() {
        let preference = DescribePreference::default();
        let other = preference.clone();
        assert_eq!(other.get().backend, Backend::Auto);
        preference.set(DescribeConfig {
            backend: Backend::Kubectl,
            kubectl_path: Some("/usr/bin/kubectl".into()),
        });
        assert_eq!(other.get().backend, Backend::Kubectl);
        assert_eq!(other.get().kubectl_path, Some("/usr/bin/kubectl".into()));
    }
}
