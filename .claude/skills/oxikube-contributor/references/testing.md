# Testing

## Layers of tests

| Layer | Test kind | Tooling |
|---|---|---|
| domain | unit + property tests (proptest) on parsers/formatters; fixture-driven view-model tests | `oxikube_testkit::fixtures` |
| ports | compile-time only (traits); fakes live in testkit | — |
| app | unit tests against `oxikube_testkit::Fake*` ports with scripted responses and recorded calls | `FakeResourcePort::script(...)`, `recorded_calls()` |
| adapters | unit tests with recorded payloads + kind integration tests behind `--features integration` | `cargo xtask kind-up` |
| platform/ui | `#[gpui::test]` with `TestApp` helpers from testkit; screenshot tests via `render_to_image` | `oxikube_testkit::gpui` |
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
- Use `TestApp::simulate_keystrokes` to drive keymaps rather than calling handlers.

## Performance checks

- `cargo xtask load-pods --context kind-oxikube --count 10000 --churn` + `oxikube --perf` logs frame times and
  feed throughput; record numbers in the PR for stories that touch tables or feeds.
