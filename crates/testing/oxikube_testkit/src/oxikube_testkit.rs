//! `oxikube_testkit` — layer: `testing`.
//!
//! Port fakes, fixtures, builders, kind helpers, gpui test helpers.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.
//!
//! # Features
//!
//! - `integration`: tests that need a live kind cluster.
//! - `screenshot`: PNG save and golden-image comparison (module `screenshot`); pure image code.
//! - `gpui-screenshot`: headless GPUI rendering to an image (module `headless`); implies `screenshot`.

#[cfg(feature = "gpui-screenshot")]
pub mod headless;
/// kind-backed integration test helpers (`OXIKUBE_TEST_CONTEXT`, `oxi-test-<rand>` namespaces).
#[cfg(feature = "integration")]
pub mod integration;
#[cfg(feature = "screenshot")]
pub mod screenshot;
