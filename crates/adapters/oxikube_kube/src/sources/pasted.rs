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

use std::collections::HashMap;
use std::path::PathBuf;

use kube::config::Kubeconfig;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::cluster_source::{ClusterSource, SourceId, SourceKind};
use oxikube_ports::secrets::{ExposeSecret, SecretString};
use oxikube_ports::{SecretKey, SecretStorePort};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use super::KubeconfigSources;
use super::layout::Layout;
use crate::kubeconfig::{Diagnostic, KubeconfigMerge, SourceStatus, is_blank_kubeconfig};

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

/// Read every pasted kubeconfig from the keychain and merge it after the files.
///
/// A secret that is missing or no longer parses becomes a [`Diagnostic`] naming the pseudo
/// path, never the text. Each pasted kubeconfig also becomes a source in `layout`. Merging goes
/// through the loader's own [`KubeconfigMerge`], so the first-wins and shadowing rules are the
/// file loader's.
pub(super) async fn fold_into(
    merge: &mut KubeconfigMerge,
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
        let key = format!("pasted:{}", descriptor.id);
        let cached = cache.lock().get(&descriptor.id).cloned();
        let text = match (cached, descriptor.secret_key()) {
            (Some(text), _) => Ok(Some(text)),
            (None, Ok(key)) => secrets.get(&key).await,
            (None, Err(err)) => Err(err),
        };
        if let Ok(Some(text)) = &text {
            cache.lock().insert(descriptor.id.clone(), text.clone());
        }
        let (status, diagnostic) = match text {
            Ok(Some(text)) => match parse(text.expose_secret()) {
                Ok(config) => {
                    merge.add_parsed(pseudo, key, config);
                    continue;
                }
                Err(_) => (
                    SourceStatus::Unparsable,
                    Diagnostic::Unparsable {
                        path: pseudo.clone(),
                    },
                ),
            },
            Ok(None) => (
                SourceStatus::Missing,
                Diagnostic::MissingFile {
                    path: pseudo.clone(),
                },
            ),
            Err(err) => (
                SourceStatus::Unreadable,
                Diagnostic::Unreadable {
                    path: pseudo.clone(),
                    reason: err.kind().to_string(),
                },
            ),
        };
        merge.add_unusable(pseudo, key, status, diagnostic);
    }
}

impl KubeconfigSources {
    /// The pasted kubeconfigs, for the caller to persist (they carry no secret).
    pub fn pasted(&self) -> Vec<PastedDescriptor> {
        self.inner.pasted.lock().clone()
    }

    /// Adds a pasted kubeconfig, reloads, and returns its descriptor.
    ///
    /// The text is validated, stored in the keychain through the [`SecretStorePort`] and kept
    /// nowhere else; the descriptor is what to persist. Pasting identical text again replaces
    /// the label. Fails with `Validation` for text that is not a kubeconfig (the message never
    /// quotes it), or with the secret store's error; nothing is written to a file either way.
    pub async fn add_pasted(&self, label: &str, text: SecretString) -> OxiResult<PastedDescriptor> {
        let descriptor = PastedDescriptor::for_text(label, text.expose_secret());
        parse(text.expose_secret())?;
        self.inner
            .secrets
            .set(&descriptor.secret_key()?, text.clone())
            .await?;
        self.inner
            .pasted_text
            .lock()
            .insert(descriptor.id.clone(), text);
        {
            let mut list = self.inner.pasted.lock();
            match list.iter_mut().find(|d| d.id == descriptor.id) {
                Some(existing) => existing.label = descriptor.label.clone(),
                None => list.push(descriptor.clone()),
            }
        }
        self.inner.reload().await?;
        Ok(descriptor)
    }

    /// Removes a pasted kubeconfig from the keychain and the catalog and reloads. Returns
    /// whether `id` was known.
    pub async fn remove_pasted(&self, id: &str) -> OxiResult<bool> {
        let Some(descriptor) = self
            .inner
            .pasted
            .lock()
            .iter()
            .find(|d| d.id == id)
            .cloned()
        else {
            return Ok(false);
        };
        // Delete the secret first: if the keychain refuses, the entry stays listed and retryable.
        self.inner.secrets.delete(&descriptor.secret_key()?).await?;
        self.inner.pasted.lock().retain(|d| d.id != id);
        self.inner.pasted_text.lock().remove(id);
        self.inner.reload().await?;
        Ok(true)
    }
}
