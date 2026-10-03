//! The shipped log filter.

use tracing_subscriber::EnvFilter;

/// Directives used when `RUST_LOG` is unset or invalid.
///
/// `info` overall, with the HTTP stack (hyper, h2, tower, rustls) capped at `warn`. Those crates
/// log request and response headers at debug and trace level; the writer-level scrub is a
/// backstop, not a reason to ship them enabled. Keep every directive here at `info` or quieter.
pub const DEFAULT_DIRECTIVES: &str =
    "info,hyper=warn,hyper_util=warn,h2=warn,tower=warn,tower_http=warn,rustls=warn";

/// `RUST_LOG` when it parses, otherwise [`DEFAULT_DIRECTIVES`].
pub fn default_filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_DIRECTIVES))
}
