# oxikube_runtime

**Layer:** `platform`

tokio <-> GPUI bridge (gpui_tokio), spawn_kube with abort-on-drop, frame-coalesced notify helpers, channels.

## Modules

- `gpui_tokio` (E05-S01): the tokio runtime as a GPUI global, ported from Zed's `gpui_tokio`
  (Apache-2.0). `init` (own 2-worker runtime), `init_from_handle` (binary-owned runtime, see
  `build_runtime`), `init_deterministic` (test mode: no runtime, no OS threads), `handle`, `mode`.
- `kube_task` (E05-S01): `spawn_kube(cx, fut) -> KubeTask<R>`, a GPUI task that aborts the tokio
  task when dropped; panics come back as `KubeTaskError::Panicked` (redacted). `live_tasks(cx)`
  counts this app's `spawn_kube` futures that have not ended (E09-S12): the leak tests assert it
  returns to its baseline after open/close cycles.
- `notify` (E05-S01): `notify_coalesced(cx)` / `cx.notify_coalesced()`, one `cx.notify()` per
  `FRAME_INTERVAL` (8.333 ms), counted by `perf::record_notify`.
- `channel` (E05-S01): `batch_channel(capacity)`, a bounded tokio channel whose receiver drains
  into an entity in batches (`BatchReceiver::drain_into`).
- The task rules (nothing blocking the UI thread, owned tasks, the self-dropping task pitfall and
  its flag + detach fix) are in the crate docs; `tests/bridge/` demonstrates each one.
- Bridge micro benchmark: `cargo run --release -p oxikube_runtime --example bridge_bench`.
- `perf` (E01-S14): the `--perf` recorder (lock-free frame ring buffer, feed and notify counters,
  and the most notifies between two frames, E07-S09),
  the JSONL flush thread (`PerfSession`), the root-view frame hook (`PerfRoot`), the process-wide
  `record_feed_deltas` / `record_notify` helpers, and (feature `perf-harness`) the scripted headless
  frame driver used by `oxikube --perf-scenario` and `cargo xtask perf`. What a frame covers and the
  file format are in docs/PERFORMANCE.md ("Perf harness"). `perf::memory` (E01-S14b) reads the
  process's RSS and peak per OS for the flush thread and the scenarios.
- Overhead: `cargo run --release -p oxikube_runtime --example perf_overhead`.
- `lazy` (E05-S13): `LazyService<T>`, a service started on first use (`ensure_init`) rather than by
- `fs` (E06-S05): `StdFs`, the `FsPort` on `std::fs` and `notify`: atomic writes (temp file + rename),
  `write_private` (created `0600`, new directories `0700`, for kubeconfigs that may hold credentials),
  `remove`, `list`, `watch`. All file work runs on tokio's blocking pool.
  an `init(cx)`, its cost logged in a `lazy_init` span and listed in the `LazyServices` global (the
  start-up checks prove none started before the first frame).
- `perf::FirstFrameProbe` and `perf::sockets` (E05-S13): the first-interactive-frame marker the
  binary wraps the main window's content in, and the count of the process's IPv4/IPv6 sockets
  behind the "no network before the first frame" check.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
