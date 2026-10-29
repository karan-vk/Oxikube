//! Reading manifests: YAML or JSON text holding any number of Kubernetes objects (E20-S01).
//!
//! The one parser behind "apply a file", "apply a directory", kustomize output and pasted
//! multi-document buffers, so each reads manifests the same way. Pure text in, values out: no
//! I/O (the app layer walks files and directories over `FsPort` and hands the text here), no
//! async runtime.
//!
//! | Module | Holds |
//! |---|---|
//! | [`source`] | [`SourceId`], [`ManifestSource`]: where a document came from |
//! | `split` | the `---` / `...` splitter that tracks the line each document starts on |
//! | `parse` | [`parse_documents`], [`ManifestDoc`], [`ManifestError`] |
//!
//! # Rules
//!
//! * Documents are separated by lines that start with `---` (optionally followed by a comment or
//!   by the first line of the document) or `...`, at column 0. Indented content, so a block
//!   scalar holding a `---`, is never a separator. LF and CRLF line ends and a leading UTF-8 BOM
//!   are accepted.
//! * Empty and comment-only documents are skipped silently.
//! * A document is a mapping with string `apiVersion` and `kind` and a `metadata.name`.
//!   `metadata.generateName` without a name is an error: `kubectl apply` rejects it, so apply
//!   refuses it up front instead of failing on the server.
//! * `kind: List` (any `*List` with `items`) and a top-level array (JSON) expand into one
//!   document per item. Items carry the line of the list they came from.
//! * One broken document is one [`ManifestError`]; the others still load. [`parse_documents`]
//!   never fails as a whole.
//! * YAML 1.1 octal integers (`defaultMode: 0644`) read as kubectl reads them.
//!
//! # Sensitivity
//!
//! A manifest can be a Secret. Neither a [`ManifestError`] nor the `Debug` of a [`ManifestDoc`]
//! holds or prints document text: errors carry a position and a generic message (the parser's
//! own message quotes the text and is dropped).

mod parse;
pub mod source;
mod split;

pub use parse::{ManifestDoc, ManifestError, parse_documents};
pub use source::{ManifestSource, SourceId};

#[cfg(test)]
mod tests;
