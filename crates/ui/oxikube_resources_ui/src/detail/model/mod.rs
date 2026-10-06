//! The detail's data, as plain Rust: what the drawer shows for one object, built from the store's
//! object (and, for kinds whose feed carries metadata only, one full read), with nothing from
//! GPUI. The view draws exactly what is in here, so these tests are tests of what is on screen.
//!
//! | File | Holds |
//! |---|---|
//! | `header` | [`Header`]: kind, name, namespace, creation time, status chip |
//! | `meta` | [`MetaEntry`] (labels, annotations), [`OwnerLink`] |
//! | `conditions` | [`ConditionRow`]: `status.conditions` as table rows |
//! | `status` | [`StatusSummary`]: `status` flattened to key/value lines with depth and size limits |
//! | `secret` | what a Secret may show: key names, never values |
//! | `rows` | [`Row`]: the Overview flattened to one list for the virtualised body |

mod conditions;
mod header;
mod meta;
mod rows;
mod secret;
mod status;

#[cfg(test)]
mod tests;

use oxikube_domain::Resource;
use oxikube_domain::ids::Gvk;

pub use conditions::{ConditionRow, conditions_of};
pub use header::{Header, StatusChip};
pub use meta::{COLLAPSED_VALUE_CHARS, MetaEntry, OwnerLink};
pub use rows::{Row, Section};
pub use secret::{is_secret, mask_secret};
pub use status::{MAX_DEPTH, MAX_LINES, MAX_VALUE, StatusLine, StatusSummary};

use oxikube_app::store::StoreObject;

/// Everything the Overview shows for one object.
#[derive(Debug, Clone, PartialEq)]
pub struct DetailModel {
    /// Kind, name, namespace, age and status.
    pub header: Header,
    /// `metadata.labels`, by key.
    pub labels: Vec<MetaEntry>,
    /// `metadata.annotations`, by key.
    pub annotations: Vec<MetaEntry>,
    /// `metadata.ownerReferences`.
    pub owners: Vec<OwnerLink>,
    /// `metadata.finalizers`.
    pub finalizers: Vec<String>,
    /// `status.conditions`.
    pub conditions: Vec<ConditionRow>,
    /// The rest of `status`, flattened.
    pub status: StatusSummary,
    /// For a Secret: the names of its data keys (never the values). `None` for other kinds.
    pub secret_keys: Option<Vec<String>>,
    /// Whether the object's `spec` and `status` are known. `false` while a metadata-only object
    /// waits for its full read: the sections that need them say so instead of looking empty.
    pub complete: bool,
}

impl DetailModel {
    /// The model of `object` of type `gvk`. `full` is the complete object when `object` itself is
    /// metadata-only or a Table row (a full read); a complete `object` is used as it is.
    pub fn build(
        object: &StoreObject,
        gvk: &Gvk,
        full: Option<&Resource>,
        chip: Option<StatusChip>,
    ) -> Self {
        let complete_resource = match object.resource() {
            Some(resource) if !resource.is_partial() => Some(resource),
            _ => full,
        };
        let meta = object.meta();
        let secret = is_secret(gvk);
        let header = Header::new(gvk, meta, chip);
        let (labels, annotations) = meta::entries(meta, secret);
        let (conditions, status) = match complete_resource {
            Some(resource) if !secret => (conditions_of(resource), StatusSummary::of(resource)),
            _ => (Vec::new(), StatusSummary::default()),
        };
        Self {
            header,
            labels,
            annotations,
            owners: meta.owner_refs.iter().map(OwnerLink::from).collect(),
            finalizers: meta.finalizers.iter().map(ToString::to_string).collect(),
            conditions,
            status,
            secret_keys: secret.then(|| complete_resource.map(secret::keys).unwrap_or_default()),
            complete: complete_resource.is_some(),
        }
    }

    /// The Overview as one flat list of rows (what the virtualised body draws).
    pub fn rows(&self) -> Vec<Row> {
        rows::flatten(self)
    }

    /// The label or annotation `key`, for the copy action.
    pub fn meta_entry(&self, key: &str, annotation: bool) -> Option<&MetaEntry> {
        let entries = if annotation {
            &self.annotations
        } else {
            &self.labels
        };
        entries.iter().find(|entry| &*entry.key == key)
    }
}
