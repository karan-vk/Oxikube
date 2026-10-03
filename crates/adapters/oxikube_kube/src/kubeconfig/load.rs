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
    match Kubeconfig::read_from(path) {
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
    let mut merged = Kubeconfig::default();
    let mut sources = Vec::with_capacity(paths.len());
    let mut origins: BTreeMap<ContextName, PathBuf> = BTreeMap::new();
    let mut diagnostics = Vec::new();
    let mut seen_keys = HashSet::new();

    for path in paths {
        let key = source_key(path);
        if !seen_keys.insert(key.clone()) {
            continue;
        }
        let mut info = SourceInfo {
            path: path.clone(),
            key,
            status: SourceStatus::Loaded,
            contexts: Vec::new(),
        };
        match load_kubeconfig_path(path) {
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
                info.status = status;
                diagnostics.push(diagnostic);
            }
            Ok(next) => {
                let names: Vec<ContextName> = next
                    .contexts
                    .iter()
                    .map(|c| ContextName::new(c.name.as_str()))
                    .collect();
                // `merge` consumes `self`, so keep a copy to fall back to on refusal.
                match merged.clone().merge(next) {
                    Ok(next_merged) => {
                        merged = next_merged;
                        for name in &names {
                            match origins.get(name) {
                                Some(winner) => diagnostics.push(Diagnostic::DuplicateContext {
                                    context: name.clone(),
                                    winner: winner.clone(),
                                    shadowed: path.clone(),
                                }),
                                None => {
                                    origins.insert(name.clone(), path.clone());
                                }
                            }
                        }
                        info.contexts = names;
                    }
                    Err(err) => {
                        info.status = SourceStatus::Incompatible;
                        diagnostics.push(Diagnostic::Incompatible {
                            path: path.clone(),
                            reason: err.to_string(),
                        });
                    }
                }
            }
        }
        sources.push(info);
    }

    dedupe_by_name(&mut merged.contexts, |c| c.name.as_str());
    dedupe_by_name(&mut merged.clusters, |c| c.name.as_str());
    dedupe_by_name(&mut merged.auth_infos, |a| a.name.as_str());

    let loaded = LoadedKubeconfig {
        merged,
        sources,
        origins,
        diagnostics,
    };
    if strictness == Strictness::RequireUsable && !loaded.has_usable_source() {
        return Err(unusable_error(&loaded));
    }
    Ok(loaded)
}

/// The error for [`Strictness::RequireUsable`] when nothing loaded.
///
/// `NotFound` when every listed path is missing (or none was listed), `Validation` when at
/// least one file exists but is unusable. The message names files, never their contents.
fn unusable_error(loaded: &LoadedKubeconfig) -> OxiError {
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
