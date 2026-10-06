# Testing

## Layers of tests

| Layer | Test kind | Tooling |
|---|---|---|
| domain | unit + property tests (proptest) on parsers/formatters; fixture-driven view-model tests | `oxikube_testkit::fixtures` |
| ports | compile-time only (traits); fakes live in testkit | — |
| app | unit tests against `oxikube_testkit::Fake*` ports with scripted responses and recorded calls | `FakeResourcePort::script(...)`, `recorded_calls()` |
| adapters | unit tests with recorded payloads + kind integration tests behind `--features integration` | `cargo xtask kind-up` |
| platform/ui | `#[gpui::test]` with `TestApp` helpers from testkit; screenshot tests via `ScreenshotApp` | `oxikube_testkit::gpui_test` (feature `gpui-test` / `gpui-screenshot`), `docs/testing-gpui.md` |
| bins | smoke test `--version`, startup without kubeconfig | CI |

## Fakes and fixtures

- Every port has a fake. If you add a port method, add it to the fake in the same PR.
- Fixtures are JSON manifests under `crates/testing/oxikube_testkit/fixtures/`; use the
  builders (`pod().running().restarts(3)`) for variations instead of new files.
- Recorded CLI/HTTP outputs (helm, aws, argocd) live next to the adapter under
  `tests/fixtures/` and are versioned by tool version in the filename.

## kind integration

```
cargo xtask kind-up            # creates cluster `oxikube` (context `kind-oxikube`) with metrics-server + sample CRD + fixtures
cargo test -p oxikube_kube --features integration
cargo xtask kind-down
```
`kind-up` is idempotent and only uses `--context kind-oxikube`. Fixtures live in
`oxikube_testkit/fixtures/` (see its README); metrics-server is pinned there.
In tests, `oxikube_testkit::integration` (feature `integration`) reads `OXIKUBE_TEST_CONTEXT`
(`test_context()` returns `None` when unset, so the test returns early) and
`TestNamespace::create(&ctx)` makes an `oxi-test-<rand>` namespace that is deleted on drop.
`cargo it` runs the integration tests of kube, app and testkit with the feature on.

Container images: `kind-up` pulls every image in `oxikube_testkit/fixtures/test-images.txt` into
the nodes first, so a fresh CI cluster starts with them present. A test names an image through
`oxikube_testkit::images` (`images::BUSYBOX`, ...), never a literal; a test fails when an
integration test or fixture uses an image missing from that file. Do not rely on an image that
merely happens to be cached on your long-lived cluster. To wait for a pod use
`common::pods::wait_started` / `wait_ready` (kube suite): image pulls are timed apart from the
start deadline and a failure prints the pod's states and events. Bulk pods that must stay
`Pending` use `pending_pod` (a scheduler nobody runs), never a `nodeSelector` no node matches:
thousands of those make the real scheduler minutes late for every other test.

Integration tests must create their own namespace (`oxi-test-<rand>`) and delete it. They
run in CI only for PRs touching `crates/adapters/**`, `crates/ports/**`, `xtask/**`, plus
nightly for everything.

## GPUI test determinism (hard rules)

- Never start OS threads (notify watchers, timers) in tests: use `init_with_dir` style
  constructors that take a `watch: false` flag. GPUI's scheduler panics with "Detected
  activity on thread ..." when a foreign thread wakes a task; it shows up only on Linux CI.
- Never `std::thread::sleep`; advance the test clock (`cx.advance_clock`) and
  `run_until_parked`.
- A `Task` stored in an entity must not be cleared from inside itself. Use a flag and
  `.detach()`.
- Screenshot tests need both `gpui/test-support` and `gpui_platform/test-support`;
  compare against goldens with a small tolerance; goldens live under `tests/goldens/<os>/`.
- Use `TestWindow::simulate_keystrokes` to drive keymaps rather than calling handlers.
- The full recipe, one worked example per helper and the "don't" list are in
  `docs/testing-gpui.md`; copy the tests in `crates/testing/oxikube_testkit/tests/gpui_harness.rs`.

## Performance checks

- `cargo xtask load-pods --context kind-oxikube --count 10000 --churn` + `oxikube --perf` logs frame times and
  feed throughput; record numbers in the PR for stories that touch tables or feeds.
- `cargo xtask perf <scenario>|--all [--check]` runs the headless scenarios against
  `docs/perf/baseline.json` (nightly gate: +20 %). A story that builds the view behind a stubbed
  scenario (table, palette, logs, editor) scripts it with `oxikube_runtime::perf::harness` and seeds
  its baseline (docs/PERFORMANCE.md, "Perf harness").
