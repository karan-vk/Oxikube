# oxikube_runtime

**Layer:** `platform`

tokio <-> GPUI bridge (gpui_tokio), spawn_kube with abort-on-drop, frame-coalesced notify helpers, channels.

## Modules

- `perf` (E01-S14): the `--perf` recorder (lock-free frame ring buffer, feed and notify counters),
  the JSONL flush thread (`PerfSession`), the root-view frame hook (`PerfRoot`), the process-wide
  `record_feed_deltas` / `record_notify` helpers, and (feature `perf-harness`) the scripted headless
  frame driver used by `oxikube --perf-scenario` and `cargo xtask perf`. What a frame covers and the
  file format are in docs/PERFORMANCE.md ("Perf harness"). `perf::memory` (E01-S14b) reads the
  process's RSS and peak per OS for the flush thread and the scenarios.
- Overhead: `cargo run --release -p oxikube_runtime --example perf_overhead`.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
