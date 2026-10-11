//! Shared helpers for the adapter's integration tests.
#![allow(dead_code)]

use std::path::PathBuf;
use std::time::Duration;

use jiff::Timestamp;
use oxikube_ports::{PromEndpoint, TimeRange};
use wiremock::MockServer;

/// A recorded response body from `tests/fixtures/<name>.json`.
pub fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.json"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// An endpoint at the mock server with a short timeout.
pub fn endpoint(server: &MockServer) -> PromEndpoint {
    PromEndpoint::new(server.uri()).with_timeout(Duration::from_secs(5))
}

/// A five-second range ending at a fixed instant.
pub fn range() -> TimeRange {
    let end = Timestamp::from_second(1_791_707_599).unwrap();
    TimeRange {
        start: Timestamp::from_second(1_791_707_595).unwrap(),
        end,
        step: Duration::from_secs(2),
    }
}
