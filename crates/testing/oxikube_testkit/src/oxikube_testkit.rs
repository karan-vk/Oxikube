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
//! - `gpui-headless`: a headless GPUI app context with the real text system (module `headless`);
//!   minimal scaffolding for the perf scenarios until E05-S11's `TestApp` lands.
//! - `gpui-screenshot`: headless GPUI rendering to an image (`headless::capture_view`); implies
//!   `screenshot` and `gpui-headless`.

#[cfg(feature = "gpui-headless")]
pub mod headless;
/// kind-backed integration test helpers (`OXIKUBE_TEST_CONTEXT`, `oxi-test-<rand>` namespaces).
#[cfg(feature = "integration")]
pub mod integration;
#[cfg(feature = "screenshot")]
pub mod screenshot;
