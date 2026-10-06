//! The payloads of the `kubeconfig::*` commands (E06-S05): which kubeconfig sources the user
//! reads, added by path or pasted.
//!
//! Paths are plain strings here: the domain does no I/O, and a command built from an MCP tool
//! call or a keymap argument is JSON. The app resolves them (`oxikube_app::sources`).

use std::fmt;

use serde::{Deserialize, Serialize};

/// A kubeconfig source to add: what the user picked or pasted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NewKubeconfigSource {
    /// What kubectl reads: `KUBECONFIG`, else `~/.kube/config`. Adds the entry back after it
    /// was removed.
    Default,
    /// One kubeconfig file the user keeps somewhere (it is read in place, never copied).
    File {
        /// The file's path.
        path: String,
    },
    /// A directory of kubeconfig files (the files directly inside it).
    Dir {
        /// The directory's path.
        path: String,
    },
    /// A kubeconfig pasted as text: validated, then stored by Oxikube as
    /// `<config dir>/kubeconfigs/<name>.yaml` (owner-only) and added as a file source.
    Pasted {
        /// What to call it; becomes the file name (letters, digits, `-`, `_` and `.`).
        name: String,
        /// The kubeconfig text. It may hold credentials: its `Debug` never shows it.
        text: PastedText,
    },
}

/// Which source to remove. For a file or directory it is the entry only: a file the user
/// owns is never deleted. A kubeconfig that Oxikube stored itself (pasted) is deleted with its
/// entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KubeconfigSourceRef {
    /// The `KUBECONFIG` / `~/.kube/config` entry.
    Default,
    /// A file source.
    File {
        /// The file's path, as listed.
        path: String,
    },
    /// A directory source.
    Dir {
        /// The directory's path, as listed.
        path: String,
    },
}

/// Kubeconfig text that may hold tokens and keys. Serialises as the plain string (a tool call
/// carries it that way) but never prints: `Debug` shows the length only, so a logged
/// [`Command`](super::Command) cannot leak it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PastedText(String);

impl PastedText {
    /// Wraps `text`.
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// The text. Do not log it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Unwraps the text.
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Debug for PastedText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<pasted kubeconfig, {} bytes>", self.0.len())
    }
}
