//! `oxikube_testkit` — layer: `testing`.
//!
//! Port fakes, fixtures, builders, kind helpers, gpui test helpers.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.
//!
//! # Always on
//!
//! - [`fakes`]: a `Fake*` for every port trait in `oxikube_ports`, with scripted
//!   responses (`fake.script().<method>.push_ok(..)`) and recorded calls
//!   (`fake.recorded_calls()`). Streams replay on a virtual [`FakeClockPort`]; nothing
//!   sleeps for real, starts an OS thread or needs tokio. All fakes are re-exported here.
//! - [`fixtures`]: 30+ realistic JSON manifests loaded lazily as domain `Resource`s
//!   (`fixtures::load("pods/crashloop.json")`, `fixtures::pod_crashloop()`).
//! - [`builders`]: `pod().running().restarts(3).build()`, `deployment().replicas(3).ready(2)`,
//!   `node().cordoned()`, ... producing the same `Resource` shape as the fixtures. The entry
//!   points are re-exported here.
//! - [`commands`]: `fixture_commands(n)`, a deterministic mix of command metadata (every view,
//!   selection shape and category) for palette, help and keymap tests.
//! - [`images`]: the container images the kind suites run (one list, pre-pulled by `kind-up`).
//! - [`script`]: the [`Script`] / [`CallLog`] / [`Timeline`] helpers the fakes share.
//! - [`scripted_feed`]: [`ScriptedFeed`], objects added / modified / deleted at chosen ticks as the
//!   `DeltaBatch`es a watch delivers (UI and store tests).
//! - [`test_ports`]: [`TestPorts`], the seeded bundle of fakes an `AppState` is built from.
//!
//! # Features
//!
//! - `integration`: tests that need a live kind cluster.
//! - `screenshot`: PNG save and golden-image comparison (module `screenshot`); pure image code.
//! - `gpui-test`: the deterministic GPUI harness for `#[gpui::test]`s (module `gpui_test`:
//!   `TestApp`, `TestWindow`); see `docs/testing-gpui.md`.
//! - `gpui-headless`: a headless GPUI app context with the real text system (module `headless`),
//!   used by the perf scenarios and by `ScreenshotApp`.
//! - `gpui-screenshot`: headless GPUI rendering to an image (`headless::capture_view`,
//!   `gpui_test::ScreenshotApp`, `gpui_test::run_golden_cases`); implies `screenshot`,
//!   `gpui-headless` and `gpui-test`.

pub mod builders;
pub mod commands;
pub mod fakes;
pub mod fixtures;
#[cfg(feature = "gpui-test")]
pub mod gpui_test;
#[cfg(feature = "gpui-headless")]
pub mod headless;
pub mod images;
/// kind-backed integration test helpers (`OXIKUBE_TEST_CONTEXT`, `oxi-test-<rand>` namespaces).
#[cfg(feature = "integration")]
pub mod integration;
#[cfg(feature = "screenshot")]
pub mod screenshot;
pub mod script;
pub mod scripted_feed;
pub mod test_ports;

pub use builders::{daemonset, deployment, job, node, pod, replicaset, resource, statefulset};
pub use fakes::*;
pub use script::{CallLog, Script, StreamGauge, Timeline, unscripted};
pub use scripted_feed::{ScriptedFeed, TICK};
pub use test_ports::TestPorts;
