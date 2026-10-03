//! Expanding the configured sources into kubeconfig files.
//!
//! The loader (E03-S01) takes a flat list of files. This turns the adapter's configuration
//! (`KUBECONFIG` or the default path, user-added files and directories) into that list, and
//! remembers which configured source each file belongs to so contexts can report their source.

use std::fs;
use std::io::ErrorKind as IoErrorKind;
use std::path::{Path, PathBuf};

use oxikube_ports::cluster_source::{ClusterSource, SourceId, SourceKind};

use super::SourcesConfig;
use crate::kubeconfig::{Diagnostic, split_kubeconfig_paths};

/// One configured source and the files it expands to, in load order.
pub(super) struct SourceEntry {
    pub(super) source: ClusterSource,
    pub(super) files: Vec<PathBuf>,
}

/// Every configured source, expanded. Built on the blocking pool: it lists directories.
pub(super) struct Layout {
    pub(super) entries: Vec<SourceEntry>,
    /// Directories that could not be listed.
    pub(super) diagnostics: Vec<Diagnostic>,
}

impl Layout {
    /// Expand `config` into sources and files (blocking).
    ///
    /// Order is load order, and the loader's first-file-wins rule follows it: the files named by
    /// `KUBECONFIG` (or the default path when that is unset or empty), then the user-added
    /// paths in the order given.
    pub(super) fn resolve(config: &SourcesConfig) -> Self {
        let mut layout = Layout {
            entries: Vec::new(),
            diagnostics: Vec::new(),
        };
        let env_paths = config
            .kubeconfig_env
            .as_deref()
            .map(split_kubeconfig_paths)
            .unwrap_or_default();
        if env_paths.is_empty() {
            if let Some(path) = &config.default_path {
                layout.push_file(
                    SourceId("default".into()),
                    SourceKind::KubeconfigFile,
                    "Default kubeconfig".into(),
                    path,
                );
            }
        } else {
            for path in &env_paths {
                layout.push_file(
                    SourceId(format!("env:{}", path.display())),
                    SourceKind::Environment,
                    format!("$KUBECONFIG: {}", path.display()),
                    path,
                );
            }
        }
        for path in &config.extra_paths {
            if path.is_dir() {
                layout.push_dir(path);
            } else {
                layout.push_file(
                    SourceId(format!("file:{}", path.display())),
                    SourceKind::KubeconfigFile,
                    path.display().to_string(),
                    path,
                );
            }
        }
        layout
    }

    fn push_file(&mut self, id: SourceId, kind: SourceKind, label: String, path: &Path) {
        self.entries.push(SourceEntry {
            source: ClusterSource {
                id,
                kind,
                label,
                path: Some(path.to_path_buf()),
            },
            files: vec![path.to_path_buf()],
        });
    }

    fn push_dir(&mut self, dir: &Path) {
        let files = match list_kubeconfig_files(dir) {
            Ok(files) => files,
            Err(err) => {
                self.diagnostics
                    .push(if err.kind() == IoErrorKind::NotFound {
                        Diagnostic::MissingFile {
                            path: dir.to_path_buf(),
                        }
                    } else {
                        Diagnostic::Unreadable {
                            path: dir.to_path_buf(),
                            reason: err.kind().to_string(),
                        }
                    });
                Vec::new()
            }
        };
        self.entries.push(SourceEntry {
            source: ClusterSource {
                id: SourceId(format!("dir:{}", dir.display())),
                kind: SourceKind::KubeconfigDir,
                label: dir.display().to_string(),
                path: Some(dir.to_path_buf()),
            },
            files,
        });
    }

    /// Adds a source that is not a file on disk (a pasted kubeconfig), after the loader ran.
    /// `pseudo_path` stands in for the file path in loader results.
    pub(super) fn push_virtual(&mut self, source: ClusterSource, pseudo_path: PathBuf) {
        self.entries.push(SourceEntry {
            source,
            files: vec![pseudo_path],
        });
    }

    /// Every file to hand to the loader, in order. Duplicates are the loader's to remove.
    pub(super) fn files(&self) -> Vec<PathBuf> {
        self.entries
            .iter()
            .flat_map(|e| e.files.iter().cloned())
            .collect()
    }

    /// The source that lists `origin` first, i.e. the one whose definition the loader used.
    pub(super) fn owner_of(&self, origin: &Path) -> Option<&SourceId> {
        self.entries
            .iter()
            .find(|e| e.files.iter().any(|f| f == origin))
            .map(|e| &e.source.id)
    }

    /// The configured sources, for the port's `sources()`.
    pub(super) fn sources(&self) -> Vec<ClusterSource> {
        self.entries.iter().map(|e| e.source.clone()).collect()
    }
}

/// The directories a watcher should observe (blocking: it stats paths).
///
/// Parent directories of single files, because editors and `kubectl config` replace a file by
/// renaming over it, which a watch on the file itself would lose; the directory itself for
/// directory sources. Paths that do not exist yet are left to the safety poll.
pub(super) fn watch_dirs(config: &SourcesConfig) -> Vec<PathBuf> {
    let layout = Layout::resolve(config);
    let mut dirs: Vec<PathBuf> = Vec::new();
    for entry in &layout.entries {
        let dir = match entry.source.kind {
            SourceKind::KubeconfigDir => entry.source.path.clone(),
            _ => entry
                .source
                .path
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf),
        };
        if let Some(dir) = dir.filter(|d| d.is_dir())
            && !dirs.contains(&dir)
        {
            dirs.push(dir);
        }
    }
    dirs
}

/// Whether a directory entry's name marks it as an editor, backup or lock file.
///
/// Hidden files and common leftovers are skipped so a directory source does not report
/// `.config.swp` or `config~` as broken kubeconfigs. There is deliberately no extension
/// allow-list: kubeconfigs are commonly named `config`, `kind-dev.yaml` or `prod.conf`.
fn is_ignored_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with('.')
        || lower.ends_with('~')
        || [".bak", ".swp", ".swx", ".tmp", ".orig", ".lock"]
            .iter()
            .any(|suffix| lower.ends_with(suffix))
}

/// The regular files directly inside `dir` that look like kubeconfigs, sorted by name.
///
/// Not recursive. Symlinks to files are followed; anything else that is not a regular file
/// (subdirectories, sockets, dangling links) is skipped.
fn list_kubeconfig_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if is_ignored_name(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let path = entry.path();
        if fs::metadata(&path).is_ok_and(|m| m.is_file()) {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}
