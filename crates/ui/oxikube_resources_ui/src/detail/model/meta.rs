//! Labels, annotations and owner references.

use std::sync::Arc;

use oxikube_domain::ids::Gvk;
use oxikube_domain::{ObjectMeta, OwnerRef};

use super::secret;

/// Values longer than this (or with a line break) show cut, with an expand toggle.
pub const COLLAPSED_VALUE_CHARS: usize = 120;

/// One label or annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaEntry {
    /// The key.
    pub key: Arc<str>,
    /// The whole value.
    pub value: Arc<str>,
    /// Whether the copy action may offer this entry (not for a value that is hidden).
    pub copyable: bool,
}

impl MetaEntry {
    fn new(key: &Arc<str>, value: &Arc<str>) -> Self {
        Self {
            key: key.clone(),
            value: value.clone(),
            copyable: true,
        }
    }

    /// Whether the value is too long, or too many lines, to show whole by default.
    pub fn expandable(&self) -> bool {
        self.value.chars().count() > COLLAPSED_VALUE_CHARS || self.value.contains('\n')
    }

    /// The value as shown collapsed: its first line, cut at [`COLLAPSED_VALUE_CHARS`] with `…`.
    pub fn collapsed(&self) -> String {
        let first = self.value.lines().next().unwrap_or("");
        if !self.expandable() {
            return first.to_owned();
        }
        let cut: String = first.chars().take(COLLAPSED_VALUE_CHARS).collect();
        format!("{cut}…")
    }

    /// The `key=value` text the copy action puts on the clipboard.
    pub fn copy_text(&self) -> String {
        format!("{}={}", self.key, self.value)
    }
}

/// One `metadata.ownerReferences` entry. It names its owner by type and name; the namespace (the
/// owner is in this object's, or is cluster-scoped) is settled by discovery when the link is
/// opened, never by matching strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerLink {
    /// The owner's type, from `apiVersion` and `kind`.
    pub gvk: Gvk,
    /// The owner's name.
    pub name: Arc<str>,
    /// Whether it is the managing controller.
    pub controller: bool,
}

impl From<&OwnerRef> for OwnerLink {
    fn from(owner: &OwnerRef) -> Self {
        Self {
            gvk: owner.gvk(),
            name: owner.name.clone(),
            controller: owner.controller,
        }
    }
}

/// The label and annotation entries of `meta`. A Secret's `last-applied-configuration`
/// annotation embeds its data, so its value is not shown (nor offered for copy).
pub(super) fn entries(meta: &ObjectMeta, secret: bool) -> (Vec<MetaEntry>, Vec<MetaEntry>) {
    let labels = meta
        .labels
        .iter()
        .map(|(key, value)| MetaEntry::new(key, value))
        .collect();
    let annotations = meta
        .annotations
        .iter()
        .map(|(key, value)| {
            if secret && secret::embeds_values(key) {
                MetaEntry {
                    key: key.clone(),
                    value: Arc::from(secret::HIDDEN),
                    copyable: false,
                }
            } else {
                MetaEntry::new(key, value)
            }
        })
        .collect();
    (labels, annotations)
}
