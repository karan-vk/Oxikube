//! `oxikube_testkit` — layer: `testing`.
//!
//! Port fakes, fixtures, builders, kind helpers, gpui test helpers.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

/// kind-backed integration test helpers (`OXIKUBE_TEST_CONTEXT`, `oxi-test-<rand>` namespaces).
#[cfg(feature = "integration")]
pub mod integration;
