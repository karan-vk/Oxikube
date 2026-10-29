//! Resource diff (E20-S09): one object compared with the same kind in another cluster or
//! namespace.
//!
//! "Is staging the same as prod?" Both objects are read in parallel ([`DiffService`], connecting a
//! cluster that is not connected), normalised ([`normalize`]: the fields the server owns, the
//! user's `diff.ignore_paths`, and for an object across namespaces its name and namespace; keys
//! sorted; a Secret's values replaced by fingerprints) and compared into a [`ResourceDiff`]: the
//! changed paths (`spec.replicas: 2 -> 3`), and for a ConfigMap or Secret each key as same,
//! changed or only on one side. The editor renders the two normalised objects side by side with
//! its line diff (`oxikube_editor::diff::ManifestDiff`).
//!
//! | File | Holds |
//! |---|---|
//! | `ignore` | [`IgnorePaths`]: pointer paths with `*` wildcards; [`IgnoreList`]: the shared handle the setting is pushed through |
//! | `normalize` | [`DiffOptions`], [`normalize`], and the helpers the editor's diff vs live shares ([`sort_keys`], [`is_secret`], [`remove_path`]) |
//! | `changes` | [`changes`]: the changed-path summary ([`PathChange`]) |
//! | `keys` | [`key_report`]: the keys of a ConfigMap or Secret ([`KeyDiff`], [`KeyStatus`]) |
//! | `result` | [`ResourceDiff`]: the two sides, the paths, the keys, the summary text |
//! | `service` | [`DiffService`]: the parallel read, connect on demand, [`compare`] |
//!
//! Read-only: nothing here writes to a cluster, so there is no guard tier. A Secret's values reach
//! neither the summary, nor the tool output, nor a log, nor the disk: they become fingerprints
//! (keyed at random once per comparison, equal exactly when the values are) before anything is
//! compared.

mod changes;
mod ignore;
mod keys;
mod normalize;
mod result;
mod service;
#[cfg(test)]
mod tests;

pub use changes::{ChangeKind, Changes, MAX_CHANGES, PathChange, changes};
pub use ignore::{DEFAULT_IGNORE_PATHS, IgnoreList, IgnorePaths};
pub use keys::{KeyDiff, KeyStatus, key_report};
pub use normalize::{
    DiffOptions, Fingerprinter, LAST_APPLIED, SECRET_FIELDS, is_secret, normalize, remove_path,
    sort_keys,
};
pub use result::{DiffSide, ResourceDiff};
pub use service::{DiffProgress, DiffRequest, DiffService, compare};
