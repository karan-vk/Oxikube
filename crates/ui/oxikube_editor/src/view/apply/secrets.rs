//! Applying a Secret that is shown as plaintext (E12-S11): the buffer's `data` values are text,
//! and the server wants base64. [`SecretEncoder`] turns one into the other on the way out; the
//! buffer, the diagnostics and the diff keep the text the user typed.
//!
//! An editor needs it when it was opened with `secret::EditDecoded` and the values are not masked
//! now ([`ResourceBinding::is_decoded`]), or when it holds a Secret the "Create secret" dialog
//! built ([`ManifestEditor::set_plaintext_secrets`]). A binary value, shown as a placeholder
//! (`(binary, 12 bytes)`), keeps the value the object had when it was read.

use std::sync::Arc;

use oxikube_app::SecretService;
use oxikube_domain::{OxiResult, Resource};

use crate::view::ManifestEditor;

/// Writes the Secrets of a buffer as the API takes them.
#[derive(Clone)]
pub(in crate::view) struct SecretEncoder {
    service: SecretService,
    /// The Secret as it was read: where a binary value's original comes from.
    original: Option<Arc<Resource>>,
}

impl SecretEncoder {
    /// The text to send for `text`: every Secret in it with base64 under `data`; any other
    /// document as it is. Parses and serialises `text`: run it off the UI thread.
    ///
    /// # Errors
    ///
    /// A validation error that names a key and never a value.
    pub(in crate::view) fn encode(&self, text: &str) -> OxiResult<String> {
        self.service.encode_manifest(text, self.original.as_deref())
    }
}

impl ManifestEditor {
    /// Marks the buffer as holding Secrets with plaintext values (the "Create secret" dialog's
    /// manifest): they are base64-encoded when the buffer is applied.
    pub fn set_plaintext_secrets(&mut self, on: bool) {
        self.plaintext_secrets = on;
    }

    /// What encodes the buffer for the server, `None` when its Secrets (if any) already hold
    /// base64.
    pub(in crate::view) fn secret_encoder(&self) -> Option<SecretEncoder> {
        let decoded = self
            .binding
            .as_ref()
            .is_some_and(crate::open::ResourceBinding::is_decoded);
        (decoded || self.plaintext_secrets).then(|| SecretEncoder {
            service: SecretService::new(),
            original: self
                .binding
                .as_ref()
                .and_then(|binding| binding.original().cloned()),
        })
    }
}
