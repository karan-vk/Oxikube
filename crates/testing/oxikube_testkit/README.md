# oxikube_testkit

**Layer:** `testing`

Port fakes, fixtures, builders, kind helpers, gpui test helpers.

## Fakes (always on)

- `oxikube_testkit::Fake*`: one fake per port trait in `oxikube_ports` (`FakeResourcePort`,
  `FakeDiscoveryPort`, `FakeTableFeedPort`, `FakeLogPort`, `FakeExecPort`, `FakeTerminalBackend`, `FakeExecStreamPort`, `FakePortForwardPort`,
  `FakeClusterSourcePort`, `FakeCloudDiscoveryPort`, `FakeMetricsPort`, `FakePromqlPort`,
  `FakeDescribePort`, `FakeHelmPort`, `FakeStatePort`, `FakeSecretStorePort`, `FakeNotifierPort`,
  `FakeUpdaterPort`, `FakeCrashReporterPort`, `FakeFsPort`, `FakeClockPort`, `FakeIntegrationPort`,
  `FakeToolPort`, `FakeContextProviderPort`, `FakeAgentPort`, `FakeAgentClient`).
- Script responses per method, then assert on calls:

  ```rust
  let fake = FakeResourcePort::new().with_objects([pod().name("web").build()]);
  fake.script().get.push_err(OxiError::forbidden("rbac"));
  // ... exercise the app ...
  assert!(fake.mutating_calls().is_empty());
  ```

  With an empty queue a fake falls back to its configured state (an in-memory store) or
  returns an `Internal` "no scripted response" error; each fake's rustdoc lists its fallbacks.
- Streams (`watch`, `table_feed`, `stream_logs`, agent `updates`) are scripted as a `Timeline`
  (items at offsets) replayed on a `FakeClockPort`; the test advances the clock
  (`clock.advance(..)`), so nothing sleeps. Fakes never start OS threads or need tokio, and work
  under GPUI's test scheduler.
- `ScriptedFeed` scripts a resource feed by tick: the initial list, then objects added, modified
  and deleted at chosen ticks, as the `DeltaBatch`es a watch delivers (`feed.install(&resources)`,
  then `resources.clock().advance(TICK)` per tick). The resource browser's end-to-end suite
  (`oxikube_resources_ui::suite`) is built on it.
- `tests/port_coverage.rs` fails when a port trait appears in `oxikube_ports` without a fake.
  Add a port method, add it to the fake in the same PR.

## Images (always on)

- `fixtures/test-images.txt` is the one list of container images the kind suites and the cluster
  fixtures run. `cargo xtask kind-up` pulls each into every node (`crictl pull`, with retries),
  so no test waits on a registry and none depends on an image cached only on a developer's cluster.
- `images::{PAUSE, PAUSE_PREVIOUS, E2E_BUSYBOX, BUSYBOX, NGINX}` name them for tests; use these,
  not literals. `tests/test_images.rs` fails when an integration test (`crates/{adapters,app,testing}/*/tests`)
  or a cluster fixture uses an image that is not in the list (images that must fail to pull, `*.invalid`
  and `does-not-exist`, are exempt).

## Fixtures and builders (always on)

- `fixtures/{pods,workloads,nodes,crds,events,helm,core}/*.json`: realistic manifests, loaded
  lazily with `fixtures::load("pods/crashloop.json")` or `fixtures::pod_crashloop()` as a domain
  `Resource`. Secret and Helm fixtures hold dummy data only.
- Builders for variations: `pod().running().restarts(3)`, `pod().crash_loop()`,
  `deployment().replicas(3).ready(2)`, `node().cordoned()`, `job().complete()`,
  `resource("test.oxikube.dev/v1", "Widget")`. They share the fixtures' defaults, and
  `tests/fixtures.rs` checks that they match the corresponding fixtures field by field.

## GPUI tests (feature `gpui-test`)

- `oxikube_testkit::gpui_test::TestApp::new(cx)` wraps the `TestAppContext` of a `#[gpui::test]`:
  `bind_keys`, `open_window` (returns a `TestWindow`), `update`, `run_until_parked`, `advance_clock`.
- `TestWindow` (derefs to gpui's `VisualTestContext`): `simulate_keystrokes`, `simulate_input`,
  `dispatch_action`, `advance_clock`, `draw_frame`, `bounds(selector)`, `read_root` / `update_root`.
- `oxikube_testkit::TestPorts::seeded()` (always on): the port fakes an `AppState` is built from
  (state, secrets, one-cluster catalog, seeded objects, one virtual clock), handles kept for
  assertions. `oxikube`'s `AppState::test` / `AppState::test_with(cx, &ports)` run the real init
  order over them.
- A workspace window (`open_workspace`) lives in `oxikube_workspace::test_support`: the testkit is
  below the UI layers and cannot name `Workspace`.
- Worked example per helper, the determinism rules and the "don't" list: `docs/testing-gpui.md`.
  The harness's own tests: `tests/gpui_harness.rs`.

## Headless GPUI

- Feature `gpui-headless`: `oxikube_testkit::headless::headless_context()` returns a
  `gpui::HeadlessAppContext` with the host's real text system and headless GPU renderer
  (deterministic scheduling, nothing shown). The perf scenarios (`oxikube --perf-scenario`, E01-S14)
  run on it; `ScreenshotApp` (below) wraps it for tests.

## Screenshots

- Feature `screenshot`: `oxikube_testkit::screenshot` saves PNGs and compares an image against a
  golden with a per-channel tolerance and a max differing-pixel ratio (pure image code, no window).
- Feature `gpui-screenshot` (implies `screenshot`, `gpui-headless` and `gpui-test`): `oxikube_testkit::headless::capture_view` draws a
  view off-screen via `Window::render_to_image` (turns on `gpui/test-support` and
  `gpui_platform/test-support`). Needs a GPU device: Metal on macOS, Vulkan (Mesa lavapipe is fine)
  on Linux. Windows are 2x device pixels.
- Goldens live under `<crate>/tests/goldens/<os>/<name>.png` (`<os>` = `std::env::consts::OS`).
  Create or refresh them with `OXIKUBE_UPDATE_GOLDENS=1 cargo test ...`, review the PNG, commit it.
  A mismatch writes `<name>.actual.png` and `<name>.diff.png` beside the golden (gitignored).
- Run: `cargo test -p oxikube_testkit --features screenshot`.
- The whole app: `OXIKUBE_SCREENSHOT=out.png cargo run -p oxikube --features screenshot`
  (never in default or release builds); smoke tests: `cargo test -p oxikube --features screenshot`.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
- `gpui_test::ScreenshotApp` drives a headless window with the same verbs as `TestApp`
  (`bind_keys`, `simulate_keystrokes`, `dispatch_action`, `advance_clock`) and `capture`s it;
  `gpui_test::run_golden_cases` is the `main` of a `harness = false` test that checks size,
  non-blank and the golden of the host OS. Example: `tests/gpui_screenshot.rs` (feature `gpui-golden`,
  nightly only, so `cargo test --workspace` never needs a GPU)
  (`tests/goldens/macos/counter_settled.png`). An OS without a golden gets structural checks only;
  add one by running the update command on that OS.
