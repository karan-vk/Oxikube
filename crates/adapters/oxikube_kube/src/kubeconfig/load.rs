// Portions derived from kdash (https://github.com/kdash-rs/kdash), `src/network/mod.rs` at commit
// c303673 (v2.1.1): `is_blank_kubeconfig`, `load_kubeconfig_path`, `load_kubeconfig_from_paths`
// and `load_local_kubeconfig`. MIT licence; the full text follows. Modifications (c) Oxikube
// contributors: results carry per-file sources, context origins and diagnostics instead of
// logging and a single error string; loading takes explicit inputs rather than reading the
// environment; duplicate-name handling and relative-path behaviour are explicit and tested.
//
// Copyright (c) 2021 Deepu K Sasidharan
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

//! The loader: read each listed file, skip what is unusable, merge the rest.
//!
//! Files are read with [`Kubeconfig::read_from`], which turns relative `certificate-authority`,
//! `client-certificate`, `client-key`, `token-file` and exec `command` paths into absolute paths
//! against the containing file's directory. Merging then happens on those already-absolute
//! values, so the merged result resolves correctly whichever file a credential came from.

use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::io::ErrorKind as IoErrorKind;
use std::path::{Path, PathBuf};

use kube::config::{Kubeconfig, KubeconfigError};
use oxikube_domain::ids::ContextName;
use oxikube_domain::{OxiError, OxiResult};

use super::diagnostics::{Diagnostic, SourceInfo, SourceStatus};
use super::split::resolve_kubeconfig_paths;
use super::{LoadedKubeconfig, Strictness};

#[cfg(test)]
mod tests;

/// True when a kubeconfig defines nothing: no current-context, clusters, users or contexts.
///
/// An empty or whitespace-only file parses to this.
pub fn is_blank_kubeconfig(config: &Kubeconfig) -> bool {
    config.current_context.is_none()
        && config.clusters.is_empty()
        && config.auth_infos.is_empty()
        && config.contexts.is_empty()
}

/// Why one file was not usable, before it is turned into a diagnostic and a source status.
enum Skip {
    Missing,
    Blank,
    Unreadable(String),
    Unparsable,
}

/// Read one kubeconfig file (blocking), classifying the ways it can be unusable.
fn load_kubeconfig_path(path: &Path) -> Result<Kubeconfig, Skip> {
    // kube resolves relative credential paths as `<file's parent>/<rel>`, which stays relative
    // to the working directory when the listed path is itself relative (`KUBECONFIG=sub/config`).
    // kubectl absolutises the file's directory first; do the same by absolutising the path.
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    match Kubeconfig::read_from(&absolute) {
        Ok(config) if is_blank_kubeconfig(&config) => Err(Skip::Blank),
        Ok(config) => Ok(config),
        Err(KubeconfigError::ReadConfig(err, _)) if err.kind() == IoErrorKind::NotFound => {
            Err(Skip::Missing)
        }
        // The io error kind names the failure; it never contains file contents.
        Err(KubeconfigError::ReadConfig(err, _)) => Err(Skip::Unreadable(err.kind().to_string())),
        // The parser's message can quote the offending line, so it is dropped.
        Err(_) => Err(Skip::Unparsable),
    }
}

/// The identity used to derive cluster ids and to de-duplicate listed paths.
fn source_key(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Drop later entries whose name was already seen, keeping the first (kubectl's rule).
///
/// [`Kubeconfig::merge`] only filters against what is already merged, so a name repeated inside
/// a single file would otherwise survive twice.
fn dedupe_by_name<T>(items: &mut Vec<T>, name: impl Fn(&T) -> &str) {
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(name(item).to_owned()));
}

/// Merges kubeconfig sources one at a time with the loader's rules.
///
/// The loader's loop body, for callers that also have sources that are not files (pasted text,
/// E03-S02): [`add_file`](Self::add_file) reads a path, [`add_parsed`](Self::add_parsed) takes
/// an already-parsed config under a pseudo path, [`add_unusable`](Self::add_unusable) records
/// a source that could not be read. The first definition of each name wins, a shadowed context
/// is reported, and a source key seen before is ignored. [`finish`](Self::finish) yields the
/// [`LoadedKubeconfig`]. Holds credentials: there is deliberately no `Debug`.
pub(crate) struct KubeconfigMerge {
    merged: Kubeconfig,
    sources: Vec<SourceInfo>,
    origins: BTreeMap<ContextName, PathBuf>,
    diagnostics: Vec<Diagnostic>,
    seen_keys: HashSet<String>,
}

impl KubeconfigMerge {
    /// An empty merge.
    pub(crate) fn new() -> Self {
        Self {
            merged: Kubeconfig::default(),
            sources: Vec::new(),
            origins: BTreeMap::new(),
            diagnostics: Vec::new(),
            seen_keys: HashSet::new(),
        }
    }

    /// Read the file at `path` (blocking) and merge it, or record why it is unusable. A path
    /// that resolves to a file already added is ignored.
    pub(crate) fn add_file(&mut self, path: &Path) {
        let key = source_key(path);
        if self.seen_keys.contains(&key) {
            return;
        }
        let path = path.to_path_buf();
        match load_kubeconfig_path(&path) {
            Ok(config) => self.add_parsed(path, key, config),
            Err(skip) => {
                let (status, diagnostic) = match skip {
                    Skip::Missing => (
                        SourceStatus::Missing,
                        Diagnostic::MissingFile { path: path.clone() },
                    ),
                    Skip::Blank => (
                        SourceStatus::Blank,
                        Diagnostic::BlankFile { path: path.clone() },
                    ),
                    Skip::Unreadable(reason) => (
                        SourceStatus::Unreadable,
                        Diagnostic::Unreadable {
                            path: path.clone(),
                            reason,
                        },
                    ),
                    Skip::Unparsable => (
                        SourceStatus::Unparsable,
                        Diagnostic::Unparsable { path: path.clone() },
                    ),
                };
                self.add_unusable(path, key, status, diagnostic);
            }
        }
    }

    /// Merge an already-parsed `config`. `path` stands in for a file path in origins and
    /// diagnostics, and `key` is the source identity hashed into its cluster ids.
    pub(crate) fn add_parsed(&mut self, path: PathBuf, key: String, config: Kubeconfig) {
        if !self.seen_keys.insert(key.clone()) {
            return;
        }
        let mut info = SourceInfo {
            path: path.clone(),
            key,
            status: SourceStatus::Loaded,
            contexts: Vec::new(),
        };
        let names: Vec<ContextName> = config
            .contexts
            .iter()
            .map(|c| ContextName::new(c.name.as_str()))
            .collect();
        // `merge` consumes `self`, so keep a copy to fall back to on refusal.
        match self.merged.clone().merge(config) {
            Ok(merged) => {
                self.merged = merged;
                for name in &names {
                    match self.origins.get(name) {
                        Some(winner) => self.diagnostics.push(Diagnostic::DuplicateContext {
                            context: name.clone(),
                            winner: winner.clone(),
                            shadowed: path.clone(),
                        }),
                        None => {
                            self.origins.insert(name.clone(), path.clone());
                        }
                    }
                }
                info.contexts = names;
            }
            Err(err) => {
                info.status = SourceStatus::Incompatible;
                self.diagnostics.push(Diagnostic::Incompatible {
                    path,
                    reason: err.to_string(),
                });
            }
        }
        self.sources.push(info);
    }

    /// Record a source that could not be used, with its status and diagnostic.
    pub(crate) fn add_unusable(
        &mut self,
        path: PathBuf,
        key: String,
        status: SourceStatus,
        diagnostic: Diagnostic,
    ) {
        if !self.seen_keys.insert(key.clone()) {
            return;
        }
        self.diagnostics.push(diagnostic);
        self.sources.push(SourceInfo {
            path,
            key,
            status,
            contexts: Vec::new(),
        });
    }

    /// The merged result. Names repeated inside one source are reduced to their first entry.
    pub(crate) fn finish(mut self) -> LoadedKubeconfig {
        dedupe_by_name(&mut self.merged.contexts, |c| c.name.as_str());
        dedupe_by_name(&mut self.merged.clusters, |c| c.name.as_str());
        dedupe_by_name(&mut self.merged.auth_infos, |a| a.name.as_str());
        LoadedKubeconfig {
            merged: self.merged,
            sources: self.sources,
            origins: self.origins,
            diagnostics: self.diagnostics,
        }
    }
}

/// Load and merge the kubeconfig files at `paths`, in order (blocking).
///
/// Call it from a blocking context, or use [`load_kubeconfig_from_paths`]. The result is a pure
/// function of the listed paths and the files' contents.
///
/// Missing, blank, unreadable, unparsable and merge-incompatible files are skipped with a
/// [`Diagnostic`]; with [`Strictness::Tolerant`] this never fails, and with
/// [`Strictness::RequireUsable`] it fails only when no file was usable. Paths that resolve to
/// the same file are loaded once, at the first position.
pub fn load_kubeconfig_from_paths_blocking(
    paths: &[PathBuf],
    strictness: Strictness,
) -> OxiResult<LoadedKubeconfig> {
    let mut merge = KubeconfigMerge::new();
    for path in paths {
        merge.add_file(path);
    }
    let loaded = merge.finish();
    if strictness == Strictness::RequireUsable && !loaded.has_usable_source() {
        return Err(unusable_error(&loaded));
    }
    Ok(loaded)
}

/// The error for [`Strictness::RequireUsable`] when nothing loaded.
///
/// `NotFound` when every listed path is missing (or none was listed), `Validation` when at
/// least one file exists but is unusable. The message names files, never their contents.
pub(super) fn unusable_error(loaded: &LoadedKubeconfig) -> OxiError {
    let only_missing = loaded
        .sources
        .iter()
        .all(|s| s.status == SourceStatus::Missing);
    if loaded.sources.is_empty() {
        return OxiError::not_found("no kubeconfig file to load");
    }
    let detail = loaded
        .diagnostics
        .iter()
        .filter(|d| {
            matches!(
                d,
                Diagnostic::MissingFile { .. }
                    | Diagnostic::BlankFile { .. }
                    | Diagnostic::Unreadable { .. }
                    | Diagnostic::Unparsable { .. }
                    | Diagnostic::Incompatible { .. }
            )
        })
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join("; ");
    if only_missing {
        OxiError::not_found(format!("no kubeconfig found: {detail}"))
    } else {
        OxiError::validation(format!("no usable kubeconfig: {detail}"))
    }
}

/// Load and merge the kubeconfig files at `paths` without blocking the caller's thread.
///
/// Runs [`load_kubeconfig_from_paths_blocking`] on tokio's blocking pool. Needs a tokio runtime.
pub async fn load_kubeconfig_from_paths(
    paths: Vec<PathBuf>,
    strictness: Strictness,
) -> OxiResult<LoadedKubeconfig> {
    tokio::task::spawn_blocking(move || load_kubeconfig_from_paths_blocking(&paths, strictness))
        .await
        .map_err(|err| OxiError::internal("kubeconfig loader task failed").with_source(err))?
}

/// Load the local kubeconfig from an explicit `KUBECONFIG` value and default path.
///
/// `kubeconfig_env` is the value of `KUBECONFIG` (the caller reads it; `None` when unset) and
/// `default_path` is `~/.kube/config` when the home directory is known. An unset or empty
/// `KUBECONFIG` falls back to the default path, as kubectl does (kdash returned "no config"
/// for an empty value). See [`resolve_kubeconfig_paths`].
pub async fn load_local_kubeconfig(
    kubeconfig_env: Option<OsString>,
    default_path: Option<PathBuf>,
    strictness: Strictness,
) -> OxiResult<LoadedKubeconfig> {
    let paths = resolve_kubeconfig_paths(kubeconfig_env.as_deref(), default_path.as_deref());
    load_kubeconfig_from_paths(paths, strictness).await
}
