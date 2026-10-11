//! The documents of a buffer (E10-S09): where each begins and ends, so a multi-document buffer
//! is applied one object at a time and a result can be shown on its document.
//!
//! The split is the spanned YAML model's (`crate::yaml`), the one the validator uses, so a
//! document here is a document there. Empty documents (a trailing `---`, comments only) are not
//! documents.

use std::ops::Range;

use oxikube_domain::FieldCause;

use super::{PlacedCauses, place_causes};
use crate::yaml::parse;

/// One document of a buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    /// Its place among the buffer's documents, counting the non-empty ones from 0.
    pub index: usize,
    /// Its bytes in the buffer, from its `---` (or first content) to its last node.
    pub range: Range<usize>,
    /// Those bytes, what is sent for it.
    pub text: String,
}

/// The non-empty documents of `text`, in buffer order. Parses `text`: run it off the UI thread.
pub fn split_documents(text: &str) -> Vec<Document> {
    let parsed = parse(text);
    parsed
        .docs()
        .iter()
        .filter(|doc| {
            doc.root()
                .is_some_and(|root| !doc.node(root).is_implicit_null())
        })
        .enumerate()
        .filter_map(|(index, doc)| {
            let range = doc.span.clone();
            Some(Document {
                index,
                text: text.get(range.clone())?.to_owned(),
                range,
            })
        })
        .collect()
}

/// [`place_causes`] for the document at `range` of the buffer `text`: the ranges are the
/// buffer's.
pub fn place_causes_in(text: &str, range: &Range<usize>, causes: &[FieldCause]) -> PlacedCauses {
    let Some(document) = text.get(range.clone()) else {
        return PlacedCauses::default();
    };
    let mut placed = place_causes(document, causes);
    for diagnostic in &mut placed.diagnostics {
        diagnostic.range.start += range.start;
        diagnostic.range.end += range.start;
    }
    placed
}
