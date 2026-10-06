//! The `kubeconfig` setting: which kubeconfig files and folders the catalog reads (E06-S05).
//!
//! ```json
//! "kubeconfig": {
//!   "sources": [
//!     { "kind": "default" },
//!     { "kind": "file", "path": "/work/prod.yaml" },
//!     { "kind": "dir", "path": "~/clusters" }
//!   ]
//! }
//! ```
//!
//! `default` stands for what kubectl reads: the files named by `KUBECONFIG`, else
//! `~/.kube/config`. Remove it to stop reading those. A `file` is read in place, a `dir` is read
//! file by file (not recursively). A leading `~` in a path is the home directory. The list is
//! replaced as a whole by the user layer (arrays do not merge), and a kubeconfig pasted in the
//! sources screen is stored under `<config dir>/kubeconfigs/` and listed here as a `file`.
//!
//! No secret belongs in this setting: only paths. The settings screen and the
//! `kubeconfig::*` commands (`oxikube_app::sources`) edit it through
//! [`update_user_settings`](crate::update_user_settings), and every edit, by hand or by the
//! screen, reaches the catalog through [`KubeconfigSettings::observe`].

use std::path::{Path, PathBuf};

use oxikube_ports::{UserSource, UserSourceKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::settings::Settings;

/// What a list entry points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KubeconfigSourceKind {
    /// What kubectl reads: `KUBECONFIG`, else `~/.kube/config`. Takes no `path`.
    Default,
    /// One kubeconfig file, read where it is.
    File,
    /// A folder of kubeconfig files (the files directly inside it).
    Dir,
}

/// One entry of `kubeconfig.sources`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KubeconfigSourceEntry {
    /// What the entry is.
    pub kind: KubeconfigSourceKind,
    /// The file or folder. A leading `~` is your home directory. Not used by `default`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl KubeconfigSourceEntry {
    /// The entry as the cluster source port takes it, with `~` expanded. `None` for a `file` or
    /// `dir` without a path (a typo in the file): such an entry is skipped, not an error.
    pub fn to_user_source(&self, home: Option<&Path>) -> Option<UserSource> {
        let kind = match self.kind {
            KubeconfigSourceKind::Default => return Some(UserSource::default_source()),
            KubeconfigSourceKind::File => UserSourceKind::File,
            KubeconfigSourceKind::Dir => UserSourceKind::Dir,
        };
        let path = expand_home(self.path.as_deref()?.trim(), home);
        (!path.as_os_str().is_empty()).then_some(UserSource {
            kind,
            path: Some(path),
        })
    }

    /// The entry for `source`, as written to the file (no `~`).
    pub fn from_user_source(source: &UserSource) -> Self {
        Self {
            kind: match source.kind {
                UserSourceKind::Default => KubeconfigSourceKind::Default,
                UserSourceKind::File => KubeconfigSourceKind::File,
                UserSourceKind::Dir => KubeconfigSourceKind::Dir,
            },
            path: source.path.as_ref().map(|p| p.display().to_string()),
        }
    }
}

/// `~` or `~/rest` become `home` or `home/rest`; anything else is unchanged.
fn expand_home(path: &str, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix('~'), home) {
        (Some(""), Some(home)) => home.to_path_buf(),
        (Some(rest), Some(home)) if rest.starts_with(['/', '\\']) => {
            home.join(rest.trim_start_matches(['/', '\\']))
        }
        _ => PathBuf::from(path),
    }
}

/// What one settings layer says about kubeconfig sources.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KubeconfigSettingsContent {
    /// The kubeconfig files and folders to read, in order: the first definition of a context
    /// name wins. `{ "kind": "default" }` is `KUBECONFIG`, else `~/.kube/config`;
    /// `{ "kind": "file", "path": "..." }` and `{ "kind": "dir", "path": "..." }` add your own.
    /// This list replaces the default list as a whole, so keep the `default` entry if you
    /// still want kubectl's files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<KubeconfigSourceEntry>>,
}

/// The resolved `kubeconfig` setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KubeconfigSettings {
    /// The entries as written.
    pub sources: Vec<KubeconfigSourceEntry>,
}

impl KubeconfigSettings {
    /// The list as the cluster source port takes it: `~` expanded, entries without a path
    /// skipped, repeats dropped (the first one counts).
    pub fn user_sources(&self) -> Vec<UserSource> {
        let home = dirs::home_dir();
        let mut out: Vec<UserSource> = Vec::new();
        for entry in &self.sources {
            if let Some(source) = entry.to_user_source(home.as_deref())
                && !out.contains(&source)
            {
                out.push(source);
            }
        }
        out
    }
}

/// What the list holds when nothing says otherwise. A test keeps it equal to `default.json`.
pub fn default_sources() -> Vec<KubeconfigSourceEntry> {
    vec![KubeconfigSourceEntry {
        kind: KubeconfigSourceKind::Default,
        path: None,
    }]
}

impl Settings for KubeconfigSettings {
    const KEY: Option<&'static str> = Some("kubeconfig");
    type Content = KubeconfigSettingsContent;

    fn from_content(content: KubeconfigSettingsContent) -> Self {
        Self {
            sources: content.sources.unwrap_or_else(default_sources),
        }
    }
}

crate::register_settings!(KubeconfigSettings);

#[cfg(test)]
mod tests;
