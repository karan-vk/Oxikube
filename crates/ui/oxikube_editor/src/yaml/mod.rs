//! The spanned YAML model (E10-S02): the buffer parsed into per-document node trees where every
//! node knows its byte span and its JSON path, for schema diagnostics (E10-S03), hover and
//! completion (E10-S05) and the diff (E10-S07).
//!
//! The text stays the source of truth: nothing here re-serialises it. Scalars keep their source
//! notation and text only (YAML 1.2 core: bare `yes` is a string); typing them is the validator's
//! job. Broken YAML still yields the tree built up to each error plus [`SyntaxDiagnostic`]s.
//!
//! Pure Rust with no gpui types, so it runs on the background executor and in plain unit tests.
//! Spans are byte ranges of the whole buffer; `oxikube_ui` converts them to editor positions.
//! granit-parser types never leave `parse.rs`.
//!
//! Layout: `parse` (granit-parser events → builder, recovery loop), `builder` (event → tree),
//! `recover` (re-sync lines), `dupes` (duplicate keys), `tree` (`DocTree` lookups), `result` (`ParseResult`), `path`
//! (`JsonPath`), `node`, `cache`.

mod builder;
mod cache;
mod dupes;
mod node;
mod parse;
mod path;
mod recover;
mod result;
mod tree;

pub use cache::ParseCache;
pub use node::{CollectionStyle, Node, NodeId, NodeKind, Role, ScalarStyle};
pub use parse::{parse, parse_shared};
pub use path::{DocPath, JsonPath, PathParseError, PathSegment};
pub use result::{KeyAt, ParseResult, SyntaxDiagnostic};
pub use tree::DocTree;
