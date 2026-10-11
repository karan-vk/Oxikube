//! JSON mode and the YAML to JSON conversion of the manifest editor (E10-S10).
//!
//! JSON is a subset of YAML 1.2 (flow style), so a JSON buffer goes through the same spanned
//! model ([`crate::yaml`]) and schema validator ([`crate::validate`]) as a YAML one. This module
//! adds what is specific to JSON, all pure and free of gpui so it runs on the background
//! executor and in plain tests:
//!
//! | Module | What |
//! |---|---|
//! | `language` | [`Language`] and [`LanguageMode`]: YAML or JSON, detected from the content or fixed |
//! | `strict` | [`first_error`]: the first place a text stops being JSON (a trailing comma, a comment, a bare word) with its span, which the YAML parser alone would accept |
//! | `convert` | [`convert`]: a buffer as the other language, with [`Warnings`] for what could not be carried over |
//! | `value` | [`Value`]: the data read from the spanned model, exact in key order and number digits |
//! | `write_json`, `write_yaml`, `quote` | the two writers; `quote` decides when a string needs quotes |
//! | `comments` | [`count_comments`]: the comments a conversion to JSON drops |
//!
//! The language of a buffer is editor state; the text stays the source of truth. A conversion is
//! an explicit command whose result replaces the buffer as one undo step.

mod comments;
mod convert;
mod error;
mod language;
mod numbers;
mod quote;
mod strict;
mod value;
mod warnings;
mod write_json;
mod write_yaml;

pub use comments::count_comments;
pub use convert::{Converted, convert, json_to_yaml, yaml_to_json};
pub use error::ConvertError;
pub use language::{Language, LanguageMode};
pub use strict::{JsonError, first_error};
pub use value::Value;
pub use warnings::Warnings;
pub use write_json::to_json;
pub use write_yaml::to_yaml;
