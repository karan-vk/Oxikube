//! `oxikube_logging` — layer: `platform`.
//!
//! tracing setup, rolling file logs, secret redaction layer, crash log hook.
//!
//! | Module | Holds |
//! |---|---|
//! | `writer` | [`RedactingMakeWriter`] / [`RedactingWriter`]: scrub every formatted line |
//! | `fields` | [`RedactingFields`]: a `FormatFields` that redacts known field names and scrubs values |
//! | `layer` | [`redacting_layer`] / [`redacting_json_layer`]: the fmt layers to install |
//! | `filter` | [`DEFAULT_DIRECTIVES`] / [`default_filter`]: the shipped log filter |
//!
//! # Secrets never reach a log sink
//!
//! The patterns live in `oxikube_domain::redact` (read its module docs for the list and the rule
//! that new secret-bearing fields must be added there). This crate applies them in two places:
//!
//! 1. **The writer is the backstop.** Third-party crates (hyper, tower-http, kube) can log
//!    headers at debug and trace level, so every event the fmt layer formats passes through
//!    [`redact`](oxikube_domain::redact::redact) before it reaches the sink. `tracing-subscriber`
//!    formats a whole event into a buffer and issues one `write_all` per event, which is what
//!    makes a per-write scrub sound; the writer relies on that and does not reassemble secrets
//!    split across writes.
//! 2. **The fields formatter redacts by name.** [`RedactingFields`] replaces the value of a field
//!    named like a secret (`token`, `id_token`, `password`, `authorization`, ...) outright and
//!    scrubs the text of every other field (a `&str` before quoting escapes its newlines). The
//!    JSON layer cannot use it (the JSON formatter visits event fields itself); it relies on the
//!    text patterns catching every sensitive name in `"<name>":<value>` form, which
//!    `oxikube_domain` tests for every name.
//!
//! Layers built here format with ANSI off: colour escapes inside a key would hide it from the
//! scrubber.
//!
//! The shipped default filter stays at `info` with the HTTP stack capped at `warn`; do not
//! enable kube's `hyper-util-tracing` feature, which logs requests.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

#![deny(missing_docs)]

mod fields;
mod filter;
mod layer;
mod writer;

pub use fields::RedactingFields;
pub use filter::{DEFAULT_DIRECTIVES, default_filter};
pub use layer::{redacting_json_layer, redacting_layer};
pub use writer::{RedactingMakeWriter, RedactingWriter};
