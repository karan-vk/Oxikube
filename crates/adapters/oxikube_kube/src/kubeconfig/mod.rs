//! Tolerant kubeconfig loading (E03-S01).
//!
//! kube-rs's `Kubeconfig::from_env` fails the whole load on the first bad path, which would
//! leave the cluster list empty over one stale file. This module loads every listed file,
//! skips the ones that are missing, blank or broken, merges the rest, and reports what it did.
//!
//! | File | Role |
//! |---|---|
//! | `split` | pure, platform-aware `KUBECONFIG` splitting and default-path resolution |
//! | `env` | the injected [`Env`], source precedence and the in-cluster decision (E03-S10) |
//! | `home` | the home-directory choice (client-go `homedir.HomeDir`), pure over an injected probe |
//! | `incluster` | the service account as a synthetic `in-cluster` context (E03-S10) |
//! | `load` | the loader (a port of kdash's, MIT; see `THIRD_PARTY_NOTICES.md`) |
//! | `diagnostics` | [`Diagnostic`], [`SourceInfo`], [`SourceStatus`] |
//!
//! # Behaviour
//!
//! * **Inputs are explicit.** Nothing here reads the process environment: callers pass the
//!   `KUBECONFIG` value and the default path (see [`load_local_kubeconfig`]). The loader does
//!   file I/O only, on tokio's blocking pool in the async entry points.
//! * **Tolerant by default.** A missing file, a blank file, an unreadable file, an unparsable
//!   file and a file whose `kind`/`apiVersion` conflicts with an earlier one are each skipped
//!   with a [`Diagnostic`]. kdash does the same; kubectl would error on unparsable files, but a
//!   desktop client must still show the clusters it can reach.
//! * **Strictness is the caller's choice.** [`Strictness::Tolerant`] never fails (the result may
//!   be empty). [`Strictness::RequireUsable`] fails only when no file was usable:
//!   [`ErrorKind::NotFound`](oxikube_domain::ErrorKind::NotFound) when nothing exists,
//!   [`ErrorKind::Validation`](oxikube_domain::ErrorKind::Validation) when files exist but none
//!   is usable.
//! * **First file wins** (kubectl's rule). `Kubeconfig::merge` in kube 4.2 appends a named
//!   cluster, user or context only if no earlier entry has that name, and keeps the first
//!   `current-context` and preferences. The loser's whole entry is discarded, not field-merged.
//!   Each shadowed context name yields a [`Diagnostic::DuplicateContext`] naming both files.
//!   Note this applies to clusters and users too: a context in file B that refers to cluster
//!   `c` uses file A's `c` if A defines one.
//! * **Source precedence** (E03-S10, see `env`): explicit sources, then `KUBECONFIG`, then
//!   `~/.kube/config`, then in-cluster. The first tier with any path is the only one loaded; the
//!   in-cluster context is a last resort, used only when no context was found and no file was
//!   broken. [`load_kubeconfig_for_env`] applies it; [`load_kubeconfig_from_paths`] and
//!   [`load_local_kubeconfig`] stay file-only. `KUBECONFIG` accepts `:` and `;` on every
//!   platform (see `split`); `~` is not expanded, as in kubectl.
//! * **Origins.** [`LoadedKubeconfig::origins`] maps each context name to the file whose
//!   definition won; [`LoadedKubeconfig::cluster_id`] derives the catalog id from it.
//! * **Relative paths.** `Kubeconfig::read_from` rewrites relative `certificate-authority`,
//!   `client-certificate`, `client-key`, `token-file` and exec `command` (when it contains a
//!   path separator) to absolute paths against the file's directory. Merge keeps those values
//!   untouched, so the merged config resolves correctly per file.
//! * **No secrets in output.** Diagnostics hold paths and context names only; parser messages
//!   are dropped because they can quote file lines. [`LoadedKubeconfig`]'s `Debug` prints
//!   counts and names, not the merged config.

mod diagnostics;
mod env;
mod home;
mod incluster;
mod load;
mod split;

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use kube::config::Kubeconfig;
use oxikube_domain::ids::{ClusterId, ContextName};

pub use diagnostics::{Diagnostic, InClusterSkip, Severity, SourceInfo, SourceStatus, SourceTier};
pub use env::{
    Env, Selection, apply_in_cluster_fallback, load_kubeconfig_for_env,
    load_kubeconfig_for_env_blocking, load_kubeconfig_for_process, select_sources,
};
pub use incluster::{
    IN_CLUSTER_CONTEXT, IN_CLUSTER_SOURCE_LABEL, IN_CLUSTER_SOURCE_PATH, apply_in_cluster_fixups,
    in_cluster_cluster_id, in_cluster_config_fixups, in_cluster_context_name,
    in_cluster_kubeconfig, in_cluster_server_url,
};
pub use load::{
    is_blank_kubeconfig, load_kubeconfig_from_paths, load_kubeconfig_from_paths_blocking,
    load_local_kubeconfig,
};
pub use split::{
    Platform, default_kubeconfig_path, resolve_kubeconfig_paths, split_kubeconfig,
    split_kubeconfig_os, split_kubeconfig_paths,
};

/// Whether a load with no usable file is an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Strictness {
    /// Never fail; an empty result carries diagnostics.
    #[default]
    Tolerant,
    /// Fail with `NotFound` or `Validation` when no file was usable.
    RequireUsable,
}

/// The result of loading the local kubeconfig files.
pub struct LoadedKubeconfig {
    /// Every usable file merged, first file wins. Holds credentials: do not log it.
    pub merged: Kubeconfig,
    /// Every consulted path in order, usable or not, de-duplicated by canonical path.
    pub sources: Vec<SourceInfo>,
    /// Which file's definition of each context name is used in [`merged`](Self::merged).
    pub origins: BTreeMap<ContextName, PathBuf>,
    /// What was skipped or shadowed, in the order it was found.
    pub diagnostics: Vec<Diagnostic>,
}

impl LoadedKubeconfig {
    /// True when at least one file loaded.
    pub fn has_usable_source(&self) -> bool {
        self.sources
            .iter()
            .any(|s| s.status == SourceStatus::Loaded)
    }

    /// The file whose definition of `context` is used.
    pub fn origin(&self, context: &ContextName) -> Option<&Path> {
        self.origins.get(context).map(PathBuf::as_path)
    }

    /// True when `context` is the synthetic in-cluster context added by the in-cluster fallback,
    /// decided by its origin ([`IN_CLUSTER_SOURCE_PATH`]), not by its name: a kubeconfig file may
    /// define a context that is also called `in-cluster`.
    pub fn is_in_cluster(&self, context: &ContextName) -> bool {
        self.origins
            .get(context)
            .is_some_and(|origin| origin == Path::new(IN_CLUSTER_SOURCE_PATH))
    }

    /// The catalog id of `context`: a hash of its origin file's canonical path and its name.
    pub fn cluster_id(&self, context: &ContextName) -> Option<ClusterId> {
        let origin = self.origins.get(context)?;
        let source = self.sources.iter().find(|s| &s.path == origin)?;
        Some(ClusterId::new(&source.key, context))
    }

    /// The context names in the merged config, in merge order.
    pub fn context_names(&self) -> impl Iterator<Item = ContextName> + '_ {
        self.merged
            .contexts
            .iter()
            .map(|c| ContextName::new(c.name.as_str()))
    }
}

impl fmt::Debug for LoadedKubeconfig {
    /// Names and counts only: the merged config holds tokens and keys.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoadedKubeconfig")
            .field("contexts", &self.origins.keys().collect::<Vec<_>>())
            .field("sources", &self.sources)
            .field("diagnostics", &self.diagnostics)
            .finish_non_exhaustive()
    }
}
