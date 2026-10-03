//! Pasted kubeconfigs: credential-bearing text that never touches a plain file.
//!
//! The story's first criterion says "pasted kubeconfigs stored under config dir", but a
//! kubeconfig holds tokens and keys and non-negotiable 5 forbids secrets on disk. The route
//! taken here: the text lives in the OS keychain through [`SecretStorePort`], under
//! [`SecretKey`] `kubeconfig-paste/<id>`. What the caller persists (in settings) is only a
//! [`PastedDescriptor`]: an id (a hash prefix of the text) and a user-chosen label. Neither
//! reveals the content. If no keychain is available, adding a pasted kubeconfig fails with the
//! store's error; there is no fallback to a file.
//!
//! The text is read from the keychain once per adapter (then held in memory, as a
//! `SecretString`, because reloads are frequent and a keychain read can be slow or prompt),
//! and parsed in memory at every reload. It is merged after the file sources, so a file
//! defining the same context name wins (the loader's first-wins rule).
//! Relative `certificate-authority`, `client-key` and similar paths in pasted text are not
//! rewritten, because there is no directory to resolve them against; use the inline `-data`
//! fields.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use kube::config::Kubeconfig;
use oxikube_domain::ids::ContextName;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::cluster_source::{ClusterSource, SourceId, SourceKind};
use oxikube_ports::secrets::{ExposeSecret, SecretString};
use oxikube_ports::{SecretKey, SecretStorePort};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use super::layout::Layout;
use crate::kubeconfig::{
    Diagnostic, LoadedKubeconfig, SourceInfo, SourceStatus, is_blank_kubeconfig,
};

/// Keychain namespace of pasted kubeconfig text.
const SECRET_NAMESPACE: &str = "kubeconfig-paste";

/// The non-secret handle of one pasted kubeconfig. Safe to persist and to log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PastedDescriptor {
    /// Stable id: 16 hex characters derived from the pasted text, so pasting the same text
    /// twice yields one entry.
    pub id: String,
    /// What the user called it. Shown as the source label.
    pub label: String,
}

impl PastedDescriptor {
    /// The descriptor for `text`, labelled `label`.
    pub(super) fn for_text(label: &str, text: &str) -> Self {
        let digest = Sha256::digest(text.as_bytes());
        Self {
            id: hex::encode(&digest[..8]),
            label: label.trim().to_owned(),
        }
    }

    /// The keychain key holding the text.
    pub(super) fn secret_key(&self) -> OxiResult<SecretKey> {
        SecretKey::new(SECRET_NAMESPACE, &self.id)
    }

    /// Stands in for a file path in loader results: unique per paste, never a real file.
    fn pseudo_path(&self) -> PathBuf {
        PathBuf::from(format!("pasted:{}", self.id))
    }

    fn source_id(&self) -> SourceId {
        SourceId(format!("pasted:{}", self.id))
    }

    fn label_or_id(&self) -> &str {
        if self.label.is_empty() {
            &self.id
        } else {
            &self.label
        }
    }
}

/// Parse pasted text, refusing anything that is not a usable kubeconfig.
///
/// The error message is fixed wording: parser messages can quote the offending line.
pub(super) fn parse(text: &str) -> OxiResult<Kubeconfig> {
    match Kubeconfig::from_yaml(text) {
        Ok(config) if is_blank_kubeconfig(&config) => {
            Err(OxiError::validation("the pasted kubeconfig is empty"))
        }
        Ok(config) => Ok(config),
        Err(_) => Err(OxiError::validation(
            "the pasted text is not a valid kubeconfig",
        )),
    }
}

/// Read every pasted kubeconfig from the keychain and merge it into `loaded`, after the files.
///
/// A secret that is missing or no longer parses becomes a [`Diagnostic`] naming the label,
/// never the text. Each pasted kubeconfig also becomes a source in `layout`.
pub(super) async fn fold_into(
    loaded: &mut LoadedKubeconfig,
    layout: &mut Layout,
    secrets: &dyn SecretStorePort,
    pasted: &[PastedDescriptor],
    cache: &Mutex<HashMap<String, SecretString>>,
) {
    for descriptor in pasted {
        let pseudo = descriptor.pseudo_path();
        layout.push_virtual(
            ClusterSource {
                id: descriptor.source_id(),
                kind: SourceKind::KubeconfigFile,
                label: format!("Pasted: {}", descriptor.label_or_id()),
                path: None,
            },
            pseudo.clone(),
        );
        let mut info = SourceInfo {
            path: pseudo.clone(),
            key: format!("pasted:{}", descriptor.id),
            status: SourceStatus::Loaded,
            contexts: Vec::new(),
        };
        let cached = cache.lock().get(&descriptor.id).cloned();
        let text = match (cached, descriptor.secret_key()) {
            (Some(text), _) => Ok(Some(text)),
            (None, Ok(key)) => secrets.get(&key).await,
            (None, Err(err)) => Err(err),
        };
        if let Ok(Some(text)) = &text {
            cache.lock().insert(descriptor.id.clone(), text.clone());
        }
        let config = match text {
            Ok(Some(text)) => parse(text.expose_secret()).map_err(|_| Diagnostic::Unparsable {
                path: pseudo.clone(),
            }),
            Ok(None) => Err(Diagnostic::MissingFile {
                path: pseudo.clone(),
            }),
            Err(err) => Err(Diagnostic::Unreadable {
                path: pseudo.clone(),
                reason: err.kind().to_string(),
            }),
        };
        match config {
            Err(diagnostic) => {
                info.status = match diagnostic {
                    Diagnostic::MissingFile { .. } => SourceStatus::Missing,
                    Diagnostic::Unreadable { .. } => SourceStatus::Unreadable,
                    _ => SourceStatus::Unparsable,
                };
                loaded.diagnostics.push(diagnostic);
            }
            Ok(config) => merge(loaded, &mut info, config),
        }
        loaded.sources.push(info);
    }
    // `Kubeconfig::merge` only filters against what was merged before, so a name repeated
    // inside one pasted text would otherwise survive twice (the file loader does the same).
    dedupe_by_name(&mut loaded.merged.contexts, |c| c.name.as_str());
    dedupe_by_name(&mut loaded.merged.clusters, |c| c.name.as_str());
    dedupe_by_name(&mut loaded.merged.auth_infos, |a| a.name.as_str());
}

/// Keep the first entry of each name.
fn dedupe_by_name<T>(items: &mut Vec<T>, name: impl Fn(&T) -> &str) {
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(name(item).to_owned()));
}

/// Merge one parsed kubeconfig into `loaded` with the loader's rules: the first definition of
/// a name wins, and a shadowed context is reported. The same logic as the file loader's loop
/// body, which works on paths and cannot take text.
fn merge(loaded: &mut LoadedKubeconfig, info: &mut SourceInfo, next: Kubeconfig) {
    let names: Vec<ContextName> = next
        .contexts
        .iter()
        .map(|c| ContextName::new(c.name.as_str()))
        .collect();
    match loaded.merged.clone().merge(next) {
        Ok(merged) => {
            loaded.merged = merged;
            for name in &names {
                match loaded.origins.get(name) {
                    Some(winner) => loaded.diagnostics.push(Diagnostic::DuplicateContext {
                        context: name.clone(),
                        winner: winner.clone(),
                        shadowed: info.path.clone(),
                    }),
                    None => {
                        loaded.origins.insert(name.clone(), info.path.clone());
                    }
                }
            }
            info.contexts = names;
        }
        Err(err) => {
            info.status = SourceStatus::Incompatible;
            loaded.diagnostics.push(Diagnostic::Incompatible {
                path: info.path.clone(),
                reason: err.to_string(),
            });
        }
    }
}
