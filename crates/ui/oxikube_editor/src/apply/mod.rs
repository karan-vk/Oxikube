//! Applying the buffer (E10-S08), the part without a window: the server's rejection mapped onto
//! the buffer ([`place_causes`]) and the preview's diff ([`preview_diff`]). The flow itself (dry
//! run, preview, conflicts, apply) is `view::apply`; the pipeline is `oxikube_app::ApplyService`.
//!
//! | Module | What |
//! |---|---|
//! | `documents` | [`split_documents`]: the buffer's documents with their byte ranges (E10-S09), [`place_causes_in`]: a cause placed on its document of the buffer |
//! | `spans` | [`place_causes`]: `Status.details.causes[].field` (validation and server-side apply notation) to a buffer range, the nearest parent when the field is missing |

mod documents;
mod spans;

use oxikube_app::ApplyPreview;

pub use documents::{Document, place_causes_in, split_documents};
pub use spans::{PlacedCauses, SERVER_CODE, place_causes};

use crate::diff::ManifestDiff;

/// What the preview shows: the live object against the object as the server would store it,
/// both normalised the way the diff vs live is (server-owned fields ignored, keys sorted, Secret
/// values fingerprinted). Everything for a new object. Allocates both objects: run it off the UI
/// thread.
pub fn preview_diff(preview: &ApplyPreview) -> ManifestDiff {
    ManifestDiff::of_manifests(preview.live.as_ref(), &preview.result)
}
