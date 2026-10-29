//! Where a manifest came from.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Names the text a document was read from: a file path, or a label such as `clipboard` or
/// `kustomize output`. Cheap to clone; shown in error messages.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(Arc<str>);

impl SourceId {
    /// A source named `label`.
    pub fn new(label: impl Into<Arc<str>>) -> Self {
        Self(label.into())
    }

    /// The source of the file at `path`.
    pub fn file(path: &Path) -> Self {
        Self(path.to_string_lossy().into())
    }

    /// The name as shown to the user.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SourceId({:?})", self.0)
    }
}

/// What the user asked to load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestSource {
    /// Text held in memory (a buffer, the clipboard, kustomize output) under `label`.
    Text {
        /// Shown in errors.
        label: SourceId,
        /// The manifest text.
        text: Arc<str>,
    },
    /// One `.yaml`, `.yml` or `.json` file.
    File(PathBuf),
    /// A directory, loaded recursively.
    Directory(PathBuf),
}

impl ManifestSource {
    /// The [`SourceId`] documents of this source are reported under (for a directory, each file
    /// has its own).
    pub fn id(&self) -> SourceId {
        match self {
            Self::Text { label, .. } => label.clone(),
            Self::File(path) | Self::Directory(path) => SourceId::file(path),
        }
    }
}
