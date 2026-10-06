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
//! | `init` | [`init`] / [`build`]: the global subscriber with rolling files (E05-S09); [`LogConfig`], [`LogGuard`], [`LogHandle`] (live filter changes) |
//! | `settings` | [`LogSettings`] / [`follow`]: the `log.filter` setting, hot reloaded |
//! | `crash` | [`install_panic_hook`]: a redacted crash report file on panic, then the previous hook |
//!
//! # Start-up
//!
//! `bins/oxikube` does this first, before GPUI exists, so everything after it can log:
//!
//! 1. [`init`] with a [`LogConfig`] for `<data dir>/oxikube/logs` (daily files, seven kept, a
//!    non-blocking writer so no log call waits for the disk), keeping the [`LogGuard`] until exit;
//! 2. [`install_panic_hook`] for `<data dir>/oxikube/crashes`;
//! 3. once the settings store is up, [`follow`] with the guard's [`LogHandle`].
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
//! Crash reports get the same scrub over the whole file (message, backtrace), and nothing is
//! uploaded anywhere.
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

mod crash;
mod fields;
mod filter;
mod init;
mod layer;
pub mod settings;
mod writer;

pub use crash::{
    CrashConfig, PanicReport, install_panic_hook, render_report as render_crash_report,
    write_report as write_crash_report,
};
pub use fields::RedactingFields;
pub use filter::{DEFAULT_DIRECTIVES, default_filter};
pub use init::{LogConfig, LogError, LogGuard, LogHandle, SetOutcome, build, init};
pub use layer::{redacting_json_layer, redacting_layer};
pub use settings::{LogSettings, LogSettingsContent, follow};
pub use writer::{RedactingMakeWriter, RedactingWriter};
