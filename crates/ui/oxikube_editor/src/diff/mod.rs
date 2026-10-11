//! Diff vs live (E10-S07): what differs between a manifest and the live object, and the
//! three-way merge when the object changes on the server while the buffer is being edited.
//!
//! Pure Rust with no gpui types, so it runs on the background executor and in plain tests, and
//! the apply preview (E10-S08), the dry-run preview (E19-S07) and the cross-cluster diff
//! (E20-S09) reuse it without the editor widget.
//!
//! | Module | What |
//! |---|---|
//! | `normalize` | [`IGNORED_FIELDS`] (the one list of server-owned fields), [`normalize`]: fields removed, keys sorted, Secret values fingerprinted |
//! | `canonical` | the buffer read as values ([`read_documents`], [`document_for`]), [`ObjectId`], the one YAML writer ([`to_yaml`], [`normalized_text`]), [`buffer_text`] |
//! | `lines` | [`ManifestDiff`]: a `similar` line diff as inline and side-by-side rows, unchanged stretches folded |
//! | `merge` | [`ThreeWayMerge`]: base / mine / server, conflicts settled per hunk ([`MergeSide`]) and laid out as [`ConflictRow`]s |
//! | `sync` | [`LiveSync`]: the Clean / Dirty / Banner / Merging state machine |
//!
//! The diff is comment- and key-order-insensitive: the buffer is parsed into values (comments
//! gone), both sides are normalised and written by the same writer with sorted keys, then
//! compared line by line. Nothing here logs or keeps a manifest's text beyond the values it
//! returns; a Secret's values never appear in a diff (fingerprints stand for them).

mod canonical;
mod lines;
mod merge;
mod normalize;
mod sync;

use std::sync::Arc;

use oxikube_domain::Resource;

pub use canonical::{
    ObjectId, ReadError, buffer_text, document_for, normalized_text, read_documents, to_yaml,
};
pub use lines::{CONTEXT_LINES, InlineRow, LineKind, ManifestDiff, SideRow};
pub use merge::{ConflictRow, MergeChunk, MergeSide, ThreeWayMerge};
pub use normalize::{IGNORED_FIELDS, SECRET_PLACEHOLDER, is_secret, normalize, sort_keys};
pub use sync::{LiveObject, LiveSync, MergeStep, ObjectVersion, SyncAction, SyncState};

/// Turns a live object into the text a buffer loaded from it holds. The default is
/// [`buffer_text`]; the open-from-resource view (E10-S06) passes its own renderer so a reload or
/// a merge writes the object the way the buffer was first written.
pub type Renderer = Arc<dyn Fn(&Resource) -> String + Send + Sync>;

/// The default [`Renderer`]: [`buffer_text`].
pub fn default_renderer() -> Renderer {
    Arc::new(buffer_text)
}
