//! Tests of the CLI adapter: parsers over recorded Helm 3.10 / 4.3 output, argv, error mapping
//! and redaction, and the process behaviour (detection, version gate, streaming, timeouts, kill
//! on drop, exit-code mapping) against a fake `helm` script.

mod args;
mod errors;
#[cfg(unix)]
mod fake;
mod parse;
#[cfg(unix)]
mod process;

use std::path::PathBuf;

/// The recorded-output directories, one per Helm major.
pub(super) const VERSIONS: &[&str] = &["helm-3.10", "helm-4.3"];

/// A recorded output: `tests/fixtures/<version>/<name>`.
pub(super) fn fixture(version: &str, name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(version)
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}
