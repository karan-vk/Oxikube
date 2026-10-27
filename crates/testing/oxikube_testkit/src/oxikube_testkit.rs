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
//! - [`contracts`]: behavioural contracts a fake and the real adapters both satisfy
//!   ([`contracts::cloud_discovery`] for `CloudDiscoveryPort`).
//! - [`discovery_sets`]: whole discovery answers for the kind registry and sidebar tests (E12-S01):
//!   a stock cluster, with the Gateway API, with CRDs, CRDs only.
//! - [`kinds`]: discovery fixtures, `ResourceKind`s as an API server reports them (`kind(..).short(..)`, `core_kinds()`, CRDs that collide).
//! - [`events_scope`]: [`ScopedEvents`] / [`EventsConnector`], a real connection's events feeds
//!   narrowed to a test's namespaces or forced onto `core/v1` (the E14 kind suites).
//! - [`images`]: the container images the kind suites run (one list, pre-pulled by `kind-up`).
//! - [`manifests`]: large generated YAML (`pod_list_yaml(5 MiB, ..)`, a `kubectl get -o yaml`
//!   dump with planted schema errors) and the schema it is checked against, for the manifest
//!   editor's perf tests and the `editor-5mb` scenario (E10-S11).
//! - [`storage`]: `pvc("data").class("fast")`, `storage_class("fast").expandable(true)`, `pod_mounting(..)`
//!   for the storage views and actions (E12-S15).
//! - [`sections`]: the section matrix (E12-S20): every kind the cluster sidebar lists, with its
//!   entry, its columns, its own row actions and a seed manifest a real API server accepts, for
//!   the suites that check every kind at once (`sections::catalog()`, `KindCase::seed`).
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
pub mod contracts;
pub mod discovery_sets;
pub mod events_scope;
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
pub mod kinds;
pub mod manifests;
pub mod overlap;
#[cfg(feature = "screenshot")]
pub mod screenshot;
pub mod script;
pub mod scripted_feed;
pub mod sections;
pub mod storage;
pub mod test_ports;

pub use builders::{
    NodeBuilder, PodBuilder, daemonset, deployment, job, node, pod, replicaset, resource,
    statefulset,
};
pub use events_scope::{EventsConnector, ScopedEvents};
pub use fakes::*;
pub use overlap::Overlap;
pub use script::{CallLog, Script, StreamGauge, Timeline, unscripted};
pub use scripted_feed::{ScriptedFeed, TICK};
pub use test_ports::TestPorts;
