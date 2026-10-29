//! What the panel loads: a path, and how to read it.

use std::path::PathBuf;

/// How the path is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    /// A manifest file, or a directory of them (recursively): `kubectl apply -f`.
    Manifests,
    /// A directory with a `kustomization.yaml`, built with the user's `kustomize`:
    /// `kubectl apply -k`.
    Kustomize,
}

/// A path to load and how.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ApplySource {
    /// How the path is read.
    pub kind: SourceKind,
    /// The file or directory.
    pub path: PathBuf,
}

impl ApplySource {
    /// A file or directory of manifests.
    pub fn manifests(path: impl Into<PathBuf>) -> Self {
        Self {
            kind: SourceKind::Manifests,
            path: path.into(),
        }
    }

    /// A kustomize directory.
    pub fn kustomize(path: impl Into<PathBuf>) -> Self {
        Self {
            kind: SourceKind::Kustomize,
            path: path.into(),
        }
    }

    /// The path as the user reads it.
    pub fn display(&self) -> String {
        self.path.display().to_string()
    }
}
