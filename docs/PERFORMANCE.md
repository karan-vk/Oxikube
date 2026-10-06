# Performance budget and how to measure it

Oxikube must feel as smooth as Zed. These are the numbers reviewers hold PRs to
(ADR 0013). Reference machine: Apple M-series laptop, 120 Hz display; Linux numbers are
measured on a mid-range x86 laptop with an integrated GPU.

## Budgets

| Area | Budget | Scenario |
|---|---|---|
| Frame time | p95 ≤ 8 ms, p99 ≤ 16 ms, no frame > 50 ms | scrolling a 10 000-row pod table under 1 %/5 s churn; typing in the editor on a 5 MB YAML; resizing panes; streaming logs at 5 000 lines/s |
| Input latency | ≤ 1 frame to visible change | keystroke in palette/filter/editor; click on a row; tab switch |
| Palette | open ≤ 1 frame; filter 2 000 entries ≤ 5 ms | command palette, `:` jump, picker |
| Startup | ≤ 400 ms cold to first interactive frame; catalog before any network; settings + keymap + theme < 30 ms on the main thread | `oxikube` launch with 3 kubeconfigs, 20 contexts ([Startup](#startup-cold-start-to-the-first-interactive-frame)) |
| Cluster open | tab interactive ≤ 200 ms after connect; first table rows ≤ 1 s after feed warm | 2 000-pod cluster |
| Main thread | 0 blocking I/O, process spawn, or lock contention > 1 ms | any |
| Memory | idle < 150 MB (2 clusters); 10 k pods < 400 MB; logs/events ring-buffered | steady state after 10 min |
| CPU idle | < 1 % with two clusters connected and no visible churn | laptop on battery |
| Terminal | 60 fps under `yes`/`htop`; resize ≤ 1 frame | local shell + exec |
| Editor | typing latency ≤ 16 ms with validation debounced; 5 MB file opens ≤ 500 ms | manifest editor |
| Agent thread | streaming markdown at 200 tokens/s without dropped frames | ACP panel |

## How to measure

- `oxikube --perf` logs per-frame times, feed throughput and `notify` counts to
  `<data dir>/oxikube/perf/*.jsonl` (`~/.local/share/oxikube/perf` on Linux,
  `~/Library/Application Support/oxikube/perf` on macOS) and prints p50/p95/p99 on exit (E01-S14);
  see [Perf harness](#perf-harness-oxikube---perf-and-cargo-xtask-perf).
- `cargo xtask load-pods --count 10000 --churn` seeds the churn scenario on kind (E01-S10); see
  [Load fixture](#load-fixture-cargo-xtask-load-pods) below. `oxikube --perf-table <context>` then
  connects that context, opens its pods table and scrolls it while `--perf` records (E07-S09); see
  [Resource table](#resource-table-10-000-pods-under-churn-e07-s09).
- `cargo xtask perf <scenario>|--all` runs scripted scenarios headless and writes a report; nightly
  CI compares against [`docs/perf/baseline.json`](perf/baseline.json) and fails on > 20 % regression
  (E01-S14).
- macOS: Instruments (Time Profiler, Metal System Trace) for stalls; Linux: `perf` + `tracy`
  via the `tracy` feature on `oxikube_runtime`.
- Memory: `oxikube --perf` writes the process's resident memory (RSS, MiB) into every JSONL tick
  and the exit summary, and `cargo xtask perf` reports and gates `rss_mib` / `peak_rss_mib` per
  scenario (E01-S14b); see [Memory (RSS)](#memory-rss). Only the `--perf` windowed run says
  anything about the budget above; leaks are checked with the GPUI `leak-detection` feature in
  tests.

## Perf harness: `oxikube --perf` and `cargo xtask perf`

Built in E01-S14. Code: `oxikube_runtime::perf` (recorder, JSONL flusher, frame hook, scripted
driver), `bins/oxikube` (`--perf`, `--perf-scenario`), `xtask/src/perf.rs` (runner, baseline
check).

### What a frame is

gpui-pre 0.3.7 has no public frame-start or frame-end callback. The platform's frame request
handler (private, in `gpui::window`) runs `Window::draw` and then `Window::present` inside one app
update, and only GPUI's `profiler` feature records draw times (it also instruments every executor
task, so it is not something to ship). Oxikube therefore wraps the window's root view in
`oxikube_runtime::perf::PerfRoot` when `--perf` is on:

- **start**: GPUI renders the root view (`PerfRoot::render`) at the start of every `draw`; root
  views are never served from the view cache;
- **end**: `PerfRoot::render` schedules an `App::defer` callback, and GPUI runs deferred callbacks
  when it flushes effects at the end of the update that is drawing, i.e. after `draw` and `present`
  returned.

So a recorded frame is request-layout, prepaint and paint of the whole tree, scene finish and the
platform `present` call (which encodes and submits the GPU command buffer). It does not include
GPU execution or display latency, and GPUI only draws when a window is invalidated: an idle window
records no frames. With `--perf` off the root is not wrapped and nothing is paid.

### `oxikube --perf`

```
oxikube --perf                        # until the window closes or Ctrl-C
oxikube --perf --perf-duration 30     # quit after 30 s
oxikube --perf --perf-dir /tmp/perf   # write elsewhere
```

The recorder's hot path is lock-free: a frame is a push into a single-producer ring buffer
(4 096 frames), feed deltas (`oxikube_runtime::perf::record_feed_deltas`) and coalesced notifies
(`record_notify`) are relaxed atomic adds. A background thread (`oxikube-perf`) drains it every
second and appends JSONL; the UI thread never touches the file. Lines:

| `kind` | Fields |
|---|---|
| `start` | `schema`, `app_version`, `os`, `arch`, `pid`, `started_unix_ms`, `flush_interval_ms`, `measures` |
| `tick` (every second) | `t_ms`, `interval_ms`, `frames_us` (every frame in the interval), `dropped_frames`, `feed_deltas`, `feed_deltas_per_s`, `notifies`, `notifies_per_s`, `max_notifies_per_frame`, `rss_mib`, `peak_rss_mib` (MiB, `null` where the OS has no reader) |
| `summary` (on exit) | `duration_ms`, `frame_count`, `frames` {`count`, `p50`, `p95`, `p99`, `max`} (ms), `dropped_frames`, `feed_deltas`, `feed_deltas_per_s`, `notifies`, `notifies_per_s`, `max_notifies_per_frame`, `rss_mib` {`count`, `p50`, `p95`, `p99`, `max`} (MiB, over the per-tick readings), `peak_rss_mib` |

On exit (window closed, `--perf-duration` elapsed, or Ctrl-C) it prints to stderr, for example:

```
oxikube --perf: 2 frames in 4.5 s: p50 0.651 ms, p95 6.343 ms, p99 6.343 ms, max 6.343 ms; dropped 0; feed 0 deltas (0.0/s); notify 0 (0.0/s, at most 0 per frame); rss p50 95.3 MiB, p95 95.3 MiB, max 95.3 MiB, peak 95.3 MiB
```

Percentiles are nearest-rank (p99 of fewer than 100 frames is the maximum). Every notification
`oxikube_runtime::notify_coalesced` delivers is counted (E05-S01), and every watch event the
resource stores apply is a feed delta (E07-S09: the stores' `StoreProbe`, a relist counting each
object it lists).

`max_notifies_per_frame` is the assertion-style figure (E07-S09): the most coalesced notifies
delivered between two consecutive frames. A streaming view is coalesced to frame cadence, so it
adds at most one per frame whatever the event rate; the figure is therefore at most the number of
streaming views on screen (the pods table, the sidebar's count badges and the Workloads overview's
tiles under churn: 2 to 3), and 1 in the `scroll-10k` scenario, which has only the table and fails
otherwise. On a display slower than 120 Hz a view can land two notifies in one frame (the coalescing
interval is one 120 Hz frame, see `notify_coalesced`); GPUI folds them into one redraw.

Recorder overhead (M-series, release, `cargo run --release -p oxikube_runtime --example
perf_overhead`): a frame push is about 3 ns and the hook's timing pair about 45 ns; a feed or
notify call costs about 0.4 ns with `--perf` off and 2.5 ns with it on. End to end, the startup
scenario's `draw_ms` with and without the hook (`--perf-no-probe`) differs by under 1 µs per frame
(p50 0.009 vs 0.008 ms, median of 15 runs).

### Memory (RSS)

Added in E01-S14b; code in `oxikube_runtime::perf::memory`. The metric is the resident set size
(RSS) of the `oxikube` process in **MiB** (1 MiB = 1 048 576 bytes; metric names end in `_mib` the
way timings end in `_ms`), plus the OS's peak (high-water mark):

| OS | Current RSS | Peak RSS |
|---|---|---|
| Linux | `VmRSS` in `/proc/self/status` (kB) | `VmHWM` in the same file |
| macOS | `task_info(MACH_TASK_BASIC_INFO).resident_size` (bytes) | `getrusage(RUSAGE_SELF).ru_maxrss` (**bytes** on macOS, KiB on Linux: converted in one place, `ru_maxrss_bytes`) |
| other | none: `null` in JSONL, metric absent from scenario samples | none |

`/proc/self/status` is read instead of `/proc/self/statm` because it needs no page-size lookup and
carries the peak. macOS needs two `unsafe` FFI calls through the `libc` crate (macOS target only,
SAFETY-commented); there is no sampler crate.

- **`oxikube --perf`**: the `oxikube-perf` flush thread takes one reading each time it drains the
  recorder (every second, plus the final drain on exit) and writes `rss_mib` and `peak_rss_mib` into
  the `tick` line; the summary has `rss_mib` {p50/p95/p99/max over those readings} and the session
  `peak_rss_mib`. The UI thread never reads memory. A tick is one syscall or one small `/proc` read,
  so the flush interval is also the sampling interval: a spike shorter than a second shows only in
  the peak.
- **Scenarios** (`cargo xtask perf`): there is no separate `memory` scenario. Every scenario that
  runs frames reports `rss_mib` (a reading after each scripted frame, taken between frames outside
  the timed update, so it never lands in `frame_ms` / `draw_ms`; p50/p95/p99 across those readings)
  and `peak_rss_mib` (high-water mark at the end of the run, one observation). As for timings, the
  report takes the median across the fresh-process samples of each statistic, and
  [`docs/perf/baseline.json`](perf/baseline.json) gates it like every other metric.
- **Headless RSS is not the windowed app's RSS.** The scenario runs on GPUI's test platform: no swap
  chain, no GPU surfaces, no windowing-system state. For the same placeholder window, a headless
  scenario sits at about 45 MiB on an M-series Mac (31 MiB before E05-S13 ran the full init order) while `oxikube --perf` in a real window reads
  about 95 MiB. Use the scenario figure only to see change against the same-runner baseline, and
  use `oxikube --perf` (window open, real clusters) for the budget in the table above (idle < 150
  MB with two clusters, 10 k pods < 400 MB). The 10 k-pod and two-cluster numbers need E04/E07; the
  placeholder app measures only the process and renderer floor today.
- **RSS is not heap.** It includes shared libraries and memory-mapped files the process touched,
  and the OS may trim it (macOS compressor, Linux reclaim) without the app freeing anything; it is
  the number users see in Activity Monitor and `top`, which is why the budget is written in it.

### `cargo xtask perf`

```
cargo xtask perf startup               # one scenario, 5 samples
cargo xtask perf --all --check         # every scenario, compare with the baseline
cargo xtask perf --all --update-baseline   # cannot be combined with --check
cargo xtask perf --from-report perf-report-Linux/report-Linux.json --update-baseline
```

It builds `oxikube --features perf-scenarios` (profile `release-fast`), then per scenario runs one
discarded warm-up process and `--samples` (default 5, nightly 7) fresh processes of
`oxikube --perf-scenario <name>`. Each sample is a cold process start. The report takes, per
metric, the **median across samples** of p50/p95/p99/max, so one noisy sample cannot fail the gate.
It is written to `<target>/perf/report-<os>.json` (`--out` to change) and printed as a table.

Scenarios run headless on GPUI's test platform with the host's real text system and headless GPU
renderer (`oxikube_testkit::headless`, feature `gpui-headless`). **Headless numbers are CPU,
layout and paint-preparation time only: there is no `present` and no GPU time.** Compare them with a
same-runner baseline, never with the absolute budgets above.

| Scenario | Status | Metrics |
|---|---|---|
| `startup` | measured | the real init order to the main window's first interactive frame (E05-S13, see [Startup](#startup-cold-start-to-the-first-interactive-frame)): `first_frame_ms` (first line of `main` to the end of the update that drew the first frame), `launch_to_first_frame_ms` (process spawn to the first-frame marker on stdout, so exec and dynamic loading are included; timed by xtask), `config_load_ms` (settings + theme + keymap on the main thread), `init_<stage>_ms` (every stage of `oxikube::startup`), `state_db_open_ms` (creating and migrating the SQLite state db, off the UI thread in the app); then `frame_ms` / `draw_ms` (120 idle redraws of the main view: hook time, and wall time of the whole update measured outside GPUI) and `rss_mib` / `peak_rss_mib` (headless resident memory after the redraws, MiB; see [Memory (RSS)](#memory-rss)) |
| `scroll-10k` (alias `table-scroll-10k`) | measured (E07-S09, see [Resource table](#resource-table-10-000-pods-under-churn-e07-s09)) | `first_rows_ms` (table created on a warm feed to the first frame showing all 10 000 pods), `frame_ms` / `draw_ms` scrolling 3 rows a frame while the feed delivers a 10-event batch a frame, `rss_mib` / `peak_rss_mib`; fails unless every batch is counted as feed deltas and `max_notifies_per_frame` ≤ 1 |
| `palette` | not available: needs E05-S11 #93, E11-S03 #158 | open time, filter of 2 000 entries |
| `logs-stream` | not available: needs E05-S11 #93, E08-S02 #120 | frame time at 5 000 lines/s |
| `editor-5mb` | not available: needs E05-S11 #93, E10-S04 #146, E10-S11 #153 | open time, typing latency |

A scenario that is not available yet prints `SKIPPED` with the stories that enable it and exits 0.
The stories that build those views replace the stub in `bins/oxikube/src/perf_scenario.rs` with a
script driven by `oxikube_runtime::perf::harness::run_frames` and record a baseline in the same PR.
The scripted path and the sample schema are covered by a `#[gpui::test]` that drives a fake feed
through the same driver (`oxikube_runtime::perf::harness`).

### Baseline and the nightly gate

[`docs/perf/baseline.json`](perf/baseline.json) holds p50/p95/p99 per metric, keyed by OS
(`linux`, `macos`) and scenario. The numbers come from the nightly's own runners
(`ubuntu-latest`, `macos-latest`), because the gate compares a runner with itself; numbers from a
laptop are not comparable with a CI VM.

`--check` fails when any p50/p95/p99 of a baselined metric is more than **+20 %** higher
(`--tolerance`) **and** higher by more than an absolute noise floor in the metric's own unit:

- `*_ms` metrics: **0.25 ms** (`--noise-floor-ms`). It stops microsecond jitter on sub-millisecond
  metrics (an idle redraw is about 0.01 ms) from failing the job; it is far below any budget in
  the table above.
- `*_mib` metrics: **8 MiB** (`--noise-floor-mib`). Memory has its own floor because the ms floor
  does not apply to it (0.25 MiB would fail on allocator noise) and +20 % of a small RSS is only a
  few MiB. The run-to-run spread of the headless startup scenario is about 0.1 to 0.3 MiB on macOS,
  so 8 MiB sits well above jitter and below 6 % of the 150 MB idle budget.

While the app is a placeholder this means only `first_frame_ms`,
`launch_to_first_frame_ms` and the memory metrics effectively gate; the floors are to be re-tuned once real views land (https://github.com/karan-vk/Oxikube/issues/411). A scenario or metric with no baseline is reported as
`MISSING` and does not fail; a scenario that has a baseline but no longer runs does fail.

The nightly `perf` job (ubuntu + macOS) runs `cargo xtask perf --all --check --samples 7`, uploads
`perf-report-<OS>` and, on failure, feeds the `nightly-failure` tracking issue.

Committed numbers (`startup`, median of 7 samples; ms for timings, MiB for memory; seeded from
nightly run 37401630806 on the story branch E05-S13, which runs the real init order), with a local
M-series laptop run for reference (not gated):

| Metric | `linux` (ubuntu-latest) p50 / p99 | `macos` (macos-latest) p50 / p99 | local M5 Max (20 samples) p50 / p99 |
|---|---|---|---|
| `launch_to_first_frame_ms` | 130.9 / 130.9 | 105.9 / 105.9 | 146.2 / 146.2 |
| `first_frame_ms` | 113.5 / 113.5 | 95.1 / 95.1 | 139.5 / 139.5 |
| `config_load_ms` (settings + theme + keymap) | 1.19 / 1.19 | 1.47 / 1.47 | 0.78 / 0.78 |
| `state_db_open_ms` (off the UI thread in the app) | 3.0 / 3.0 | 4.1 / 4.1 | 2.6 / 2.6 |
| `frame_ms` (idle redraw, hook) | 0.136 / 0.178 | 0.066 / 0.294 | 0.037 / 0.044 |
| `draw_ms` (idle redraw, outside) | 0.150 / 0.192 | 0.071 / 0.322 | 0.039 / 0.047 |
| `rss_mib` (headless, after the redraws) | 122.4 / 122.4 | 39.7 / 39.7 | 44.9 / 44.9 |
| `peak_rss_mib` (headless) | 122.4 / 122.4 | 39.7 / 39.7 | 44.9 / 44.9 |

The `init_<stage>_ms` breakdown is baselined too (`docs/perf/baseline.json`). On the macOS runner the
component library's `init` (`init_ui_ms`, the font enumeration) and the platform (`init_assets_ms`)
dominate as on the laptop; on the Linux runner (lavapipe) they are 3-4 ms and opening the window with
its first draw (`init_window_ms`, about 119 ms) is the whole cost.

For scale: `oxikube --perf` with a real window on the same laptop reads about 95 MiB RSS, two to three times
the headless figure, which is why the headless number is only a regression signal. The Linux
runner's headless figure is about three times the macOS runner's (different renderer and system
libraries; not investigated further), another reason baselines are per OS.

Since E05-S13 the `startup` scenario runs the real init order (logging, platform, runtime,
settings, theme, keymap, component library, state db, `AppState`, workspace, features, keymap
re-bind, window); the baselines above were refreshed from that story's nightly artifacts (rule 2
below).

Rules for updating the baseline:

1. A PR that adds a scenario (or a metric) seeds it: dispatch the nightly on the branch
   (`gh workflow run nightly.yml --ref <branch>`), download both `perf-report-<OS>` artifacts and
   run `cargo xtask perf --from-report <file> --update-baseline` for each. Commit the result.
2. A PR that makes something intentionally slower (or much faster) refreshes the affected
   scenarios the same way and says so, with before/after numbers, in its Performance section.
3. Never edit numbers by hand and never refresh the baseline to make a red nightly green without
   explaining the regression.


## Startup: cold start to the first interactive frame

Built in E05-S13 (ADR 0013). Budgets: **≤ 400 ms** from launch to the first interactive frame with
3 kubeconfigs and 20 contexts, **< 30 ms** for loading settings, keymap and theme on the main thread,
and **no network before the first frame**.

- **The marker.** The main window's content is wrapped in `oxikube_runtime::perf::FirstFrameProbe`,
  which calls `oxikube::startup::first_frame::mark` at the end of the update that drew the first
  frame (after `draw` and `present`; from then on the window dispatches input). It records the time
  since the first line of `main` in the `StartupReport`, counts the process's IPv4/IPv6 sockets
  (`oxikube_runtime::perf::sockets`: `/proc/self/fd` + `/proc/self/net/*` on Linux, `fstat` +
  `getsockname` on macOS) and logs `first interactive frame` with both and the config-load cost.
  `oxikube --perf` also prints it with every stage's cost:

  ```
  oxikube --perf: first interactive frame after 241.9 ms (budget 400 ms); settings+theme+keymap 3.1 ms (budget 30 ms); network sockets before it: 0; init: logging 0.68 assets 105.06 runtime 0.05 settings 1.28 theme 1.12 keymap 0.65 ui 71.93 state_db 0.01 app_state 0.00 workspace 0.02 features 0.00 keymap_rebind 0.04 other 61.05 ms
  ```

  (`other`: the platform run loop starting and, on macOS, the window and its first draw, which
  happen before the window stage is recorded.)
- **Nothing waits.** The state db opens on the background executor (`LazyState`) and the main
  window opens at once with the startup placeholder: the workspace's default layout, interactive,
  marked "Restoring layout…" in the title bar while the saved layout is read through the async
  `StatePort` (`oxikube_workspace::window::open_main_window_restoring`). The saved layout replaces
  it when the read completes; a failed read leaves the default layout in place and usable. The menu
  bar is installed right after the first frame (about 19 ms with AppKit).
- **Lazy init rule.** An `init(cx)` only registers. The extension host, API and cloud discovery,
  Prometheus detection, the agent registry, the update checker and kubeconfig parsing start on first
  use through `oxikube_runtime::LazyService::ensure_init` (list: `oxikube::startup::deferred`), with
  their heavy part on `spawn_kube` or the background executor. The startup scenario fails if one has
  started (or a socket is open) at the first frame.
- **`cargo xtask perf startup`** runs the scenario in fresh processes with `KUBECONFIG` set to the
  reference fixture (3 kubeconfigs, 20 contexts, written to `<target>/perf/fixture/`), reports each
  single-observation metric's distribution across the launches (`launches` in the report), and
  checks the budgets on the p95 across launches: `launch_to_first_frame_ms` ≤ 400 ms (the run fails
  above +20 %, the nightly tolerance; between the two the row is `OVER`) and `config_load_ms` ≤ 30 ms
  (strict). `--skip-budgets` reports without failing. Headless numbers are a lower bound of the
  windowed ones (no present, no GPU), so the real-window figure below is the one the budget is about.
- **Cold, not warm.** Every sample is a fresh process (no in-process caches; the first launch after a
  build also pays code-signature checks and dynamic loading). The OS file cache stays warm: clearing
  it (`sudo purge` on macOS, `echo 3 > /proc/sys/vm/drop_caches` on Linux) needs root and is not part
  of the harness.

Measured on an Apple M5 Max (`release-fast`, 20 launches each, other builds running on the machine):

| | p50 | p95 | max |
|---|---|---|---|
| windowed: process spawn to first interactive frame | 273–280 ms | 311–348 ms | 595–630 ms (the first launch after the build) |
| windowed: first line of `main` to first interactive frame | 258–265 ms | 282–326 ms | 299–338 ms |
| windowed: settings + theme + keymap | 3.3–3.6 ms | 4.0–41.9 ms | 4.4–79 ms (two outliers in one batch of 20, load average 12; 12 more launches all 2.9–5.9 ms) |
| headless (`cargo xtask perf startup --samples 20`): spawn to first frame | 146 ms | 205 ms | 214 ms |
| headless: settings + theme + keymap | 0.78 ms | 1.17 ms | 1.17 ms |
| network sockets open at the first frame | 0 in every launch | | |

Where the time goes (windowed p50): the platform (`assets`: app object, GPU device, text system)
84–108 ms, the component library (`ui`) 72–81 ms of which about 56 ms is gpui-component enumerating
the installed fonts (it resolves the default UI and monospace families on every launch; the font
list is not cached by CoreText, so it cannot be warmed in the background), the window and its first
draw about 61–65 ms, everything else under 5 ms. The per-stage table is in the module docs of
`oxikube::startup`.

## Resource table: 10 000 pods under churn (E07-S09)

The epic's exit criterion (E07): 10 000 pods with churn scroll at ≥ 55 fps on the reference machine
with a frame-time log, first rows < 1 s after the feed is warm, inside the frame budget above.

### Measuring it

Windowed, against kind (the budget measurement):

```
cargo xtask load-pods --count 10000 --namespaces 8 --namespace oxi-<you>-load --churn  # keep running
cargo build -p oxikube --profile release-fast
target/release-fast/oxikube --perf --perf-duration 60 --perf-table kind-oxikube        # --perf-scroll 3
cargo xtask load-pods --namespace oxi-<you>-load --cleanup
```

`--perf-table <context>` (implies `--perf`) does what a user does, through the same commands: once
the catalog lists the context it runs `cluster::Connect` on the command bus (the catalog's Enter),
waits for the session, runs `resource::OpenList` for pods (the sidebar's Pods entry), waits for
the table to list, then scrolls it `--perf-scroll` rows (default 3, a fast trackpad fling) every
8.3 ms, to the end and back, until the session ends. Code: `bins/oxikube/src/perf_mode/drive.rs`.
It logs each step and how long the table took to list.

Headless, on the fake feed generator (the CI regression gate, no cluster):

```
cargo xtask perf scroll-10k            # alias: table-scroll-10k
```

The scenario (`bins/oxikube/src/perf_scenario/scroll_10k.rs`) runs the real store, store runtime
and `ResourceTable` on testkit fakes: 10 000 pods listed into a warm store, then 120 scripted frames
(1 s at 120 Hz; each draws twice, once for the feed's coalesced notify and once for the scroll, so
240 frames are measured) scrolling 3 rows each while the feed delivers one 10-event batch per frame
(6 modifies, 2 deletes, 2 creates: 1 200 events/s, about twenty times the load-pods churn). It
fails unless every batch is counted as feed deltas and no frame absorbed more than one coalesced
notify. Budgets checked by `cargo xtask perf`: `first_rows_ms` < 1 000 and, on macOS, `frame_ms`
p95 ≤ 18.2 ms (55 fps); being headless they are lower bounds of the windowed figures. The Linux
runner draws these frames with Mesa's software renderer at about 340 ms each, so the frame budget is
not checked there (its baseline still gates regressions; why it is that slow is #509).

### Numbers

Reference machine: Apple M5 Max (18 cores), macOS, built-in display, `release-fast`, story branch
`story/E07-S09-perf-tuning` on `a1d3f1c`; the shared kind cluster (one node, v1.37) with 10 000
unscheduled (`Pending`) load pods in 8 namespaces and `--churn` running; other builds running on
the machine (load average 4 to 26), so treat single digits of a percent as noise.

| Run | Frames | p50 | p95 | p99 | max | Other |
|---|---|---|---|---|---|---|
| windowed, `--perf-table`, 60 s, scrolling 3 rows / 8.3 ms | 6 019 in 60.3 s (100 fps, paced by the scroll timer) | 3.42 ms | 4.00 ms | 4.26 ms | 7.86 ms | 9 931 pods listed 258 ms after the table opened (a cold list from the API server); feed 11 430 deltas; 691 notifies, at most 3 per frame (table, sidebar badges, overview tiles); dropped 0 |
| windowed, table still (`--perf-scroll 0`), 25 s | 293 | 3.96 ms | 4.64 ms | 7.22 ms | 8.20 ms | redraws only for churn and ages |
| headless `scroll-10k`, nightly `macos-latest` (run 37492117478, the committed baseline) | 240 per launch | 3.32 ms | 4.37 ms | 5.03 ms | 5.52 ms | `first_rows_ms` 26.4 (p95 30.7 across launches); RSS 176 MiB |
| headless `scroll-10k`, nightly `ubuntu-latest` (same run, lavapipe) | 240 per launch | 338 ms | 343 ms | 348 ms | 370 ms | `first_rows_ms` 466 ms; RSS 269 MiB; see #509 |
| headless `scroll-10k` (`cargo xtask perf scroll-10k --samples 7`), medians | 240 per launch | 1.89 ms | 2.52 ms | 2.97 ms | 3.07 ms | `first_rows_ms` 25.6 (p95 29.5 across launches); 1 200 feed deltas, 120 notifies, at most 1 per frame; RSS 179 MiB |

After rebasing onto the detail drawer, row actions and states stories (`e1ba85b`) the headless
scenario reads the same (3 launches: `frame_ms` p50 1.99 ms, p95 2.88 ms; `first_rows_ms` 26.0).

So the table holds ≥ 55 fps with a wide margin (the frame p95 of 4 ms would sustain 250 fps), meets
p95 ≤ 8 ms, p99 ≤ 16 ms and no frame > 50 ms, and shows the first rows well under 1 s even from a
cold list.

Before and after the tuning below (same binary and scenario, the cell fast path switched off and
on, two runs each, `/usr/bin/time -l`): instructions retired for the whole `scroll-10k` process
59.3 G → 48.9 G, about 46 M → 37 M per frame (−20 %); headless `frame_ms` p50 4.2–4.4 → 3.3–3.9 ms
(machine load average about 20 at the time). The `table_bench` example
(`cargo run -p oxikube_resources_ui --profile release-fast --example table_bench`, two draws per
frame) went from 135 M to 108 M instructions per frame.

Store side (`cargo bench -p oxikube_app --bench store_apply`, on the feed's task, never the UI
thread): a 500-event batch into 10 000 objects with two subscribers, median 1.98 ms, p95 2.31 ms;
subscribing or re-sorting costs the caller 0.14 ms (median), the seeding task 2.76 ms.

Main thread (macOS `sample`, 8 s of the windowed run while scrolling under churn): 55 % idle, 42 %
in `Window::draw`, of which about half is laying out the visible rows (gpui-component's per-row
horizontal virtual list and taffy); the cell path (`TextCell`, `CellCache`, the provider) is about
6 % of the draw and applying the store's deltas under 0.1 %. No sample waits on a mutex or condvar
on the main thread: no lock contention.

### What was tuned

1. **Coalesced deltas** (E04-S02, E07-S01): feeds batch watch events, the store applies a batch
   once and hands each subscriber everything pending as one delta per poll; checked by
   `store::tests::churn` (500-event batches into 10 000 objects: one item, bounded apply time).
2. **Incremental sort** (E07-S01): binary-search inserts into each subscriber's sorted index, a
   bulk change re-sorted off the UI thread; the same test checks the result against a full sort.
3. **Notify at frame cadence**: the table drains every ready delta in one update and redraws
   through `notify_coalesced`; `table::tests::coalesce` (1 000 deltas inside one frame: one notify,
   one render) and `max_notifies_per_frame` check it.
4. **Cheap rows** (this story): cells go to the table as `oxikube_ui::table::TextCell`s, which the
   table draws without the delegate's element and with GPUI's ellipsis only when the text does not
   fit its column (a truncating text re-shapes on every layout pass and cannot reuse its measured
   size); the fit is checked once per cell through the text system's line layout cache. The
   visible cells' text and tone are kept between frames in `CellCache` (keyed by object version
   and column, dropped when the second turns, since ages move) so a frame re-reads only changed
   rows. A side effect: right-aligned columns (Ready, Restarts) are now right-aligned; the old
   cell element filled its column, so the alignment never showed.
5. **No layout-dependent work per row**: none was found; the fit check reads cached line layouts.

### Memory: over budget (follow-up)

The windowed app connected to the cluster with the 10 000 pods listed reads **504 MiB** RSS (idle
app 117 MiB), over the 400 MB budget for 10 k pods; the headless scenario (testkit pods, one
store) reads 179 MiB against 45 MiB for `startup`. It does not grow: over the 60 s windowed run RSS
stayed between 504.0 and 504.2 MiB. The pods are held twice as `serde_json::Value`
trees (the kube adapter's reflector store and the resource store's cache, each a full `Resource`
with `managedFields` stripped; a load pod is about 2.1 KB of JSON), and a `Value` tree costs many
times its JSON. Halving it means sharing one object between the reflector store and the store
(a port change), and the budget probably needs the store to keep only column-relevant fields hot
and the JSON compact or lazy. Both are store/adapter changes beyond this story: tracked in
[#508](https://github.com/karan-vk/Oxikube/issues/508).

## Load fixture: `cargo xtask load-pods`

The perf fixture for the E07 table/feed stories and the E01-S14 harness. It creates pause pods
(`registry.k8s.io/pause:3.10`, 1m CPU / 1Mi requests, label `app=oxikube-load`) against the local
kind cluster from `cargo xtask kind-up`.

```
cargo xtask kind-up
cargo xtask load-pods --count 10000 --namespaces 8 --churn   # the budget scenario
cargo xtask load-pods --cleanup                              # delete the oxikube-load-* namespaces
```

On a cluster other people or tests use (the shared `kind-oxikube` of the agent teams), give the
load its own prefix (`--namespace oxi-<you>-load`) and clean it up with the same prefix afterwards.

| Flag | Default | Meaning |
|---|---|---|
| `--count N` | 1000 | pods to create, named `load-0 .. load-(N-1)` |
| `--namespaces N` | 4 | spread round-robin over `<prefix>-0 .. <prefix>-(N-1)` |
| `--namespace PREFIX` | `oxikube-load` | namespace name prefix |
| `--churn` | off | every 5 s delete and recreate about 1 % of the pods (min 1), cycling through all namespaces, until Ctrl-C |
| `--schedule` | off | let the cluster's scheduler place the pods; without it they carry `schedulerName: oxikube-load-unscheduled`, which no scheduler runs, so they stay `Pending` and the real scheduler never queues them (E07-S09) |
| `--cleanup` | | delete the load namespaces (only those labelled `app=oxikube-load` and named `<prefix>-<digits>`) and exit |
| `--context C` | `kind-oxikube` | kubectl context; anything not starting with `kind-` is refused |
| `--allow-non-kind` | off | override the guard (you almost certainly do not want this) |

Every kubectl call carries `--context`, so the tool never follows your current context. Pods are
applied in chunks of 500 with progress output (about 120 pods/s on a laptop, so 10 000 pods take
roughly 90 s). Ctrl-C stops the churn loop after the current tick, recreates whatever that tick
deleted so the fleet is whole, and exits 0 (kubectl children run in their own process group, so the
terminal's SIGINT does not kill an in-flight call); a second Ctrl-C aborts at once. The tool is idempotent:
re-running it with the same flags re-applies the same pods.

### What kind can actually hold

By default (no `--schedule`) every load pod is `Pending` by design: thousands of pods the scheduler
cannot place make it minutes late for every other pod on the cluster (#484), which on a shared
cluster starves everyone's tests. Pass `--schedule` only on a cluster of your own; then:

A default single-node kind cluster allows 110 pods per node. Measured with `--count 2000`: 96
Running, 1 904 Pending, because the scheduler cannot place the rest. Pending pods are still real
rows, still stream watch events and still exercise churn, sorting, filtering and grouping, but they
do not exercise running-pod columns (restarts, node, IP, metrics). **10 000 pods on the default
kind cluster is therefore mostly Pending, not Running**; quote it as such in perf reports.

To get more Running pods, create the cluster with several workers and a raised kubelet `maxPods`
(this config was verified: 3 workers x 250 gave 744 Running of 1 000 pods; the control-plane node is
tainted and takes none):

```yaml
# kind-load.yaml
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
nodes:
  - role: control-plane
  - role: worker
  - role: worker
  - role: worker
kubeadmConfigPatches:
  - |
    kind: KubeletConfiguration
    maxPods: 250
```

```
kind create cluster --name oxikube --config kind-load.yaml --wait 180s && cargo xtask kind-up
```

`maxPods: 250` is about the ceiling per node because each kind node gets a /24 pod CIDR (256
addresses); 10 000 Running pods would need roughly 40 such workers, which is not realistic on a
laptop or a CI runner. A fake-node tool (KWOK) is the route if a story ever needs 10 k Running pods;
no such number has been verified here. Do not put "10 k Running" in a report without measuring it.

## Rules that keep us inside the budget

1. Never block the UI thread: all I/O in `oxikube_app`/adapters on Tokio via
   `oxikube_runtime::spawn_kube`; results are posted as batched deltas.
2. Coalesce notifications: `notify_coalesced` batches `cx.notify()` to frame cadence; feeds
   batch watch events (E04-S02).
3. Virtualise everything that scrolls: `uniform_list`, `oxikube_ui::Table`, virtual log and
   thread views; never build off-screen rows.
4. Incremental state: `ResourceStore` applies deltas and keeps sorted indices; no full
   re-sort per event; filters are incremental.
5. Cache layout-heavy artefacts: shaped text runs for log lines, parsed YAML trees, theme
   tokens, icons; invalidate by key.
6. Keep hot-path allocations out: reuse buffers in feeds, logs and terminal grids; measure
   with `--perf` allocation counters.
7. Lazy and idle: feeds start on demand and stop when unobserved (watch budget); metrics
   polling pauses for hidden tabs; inactive windows render at reduced rate (GPUI default).
8. Animations are short (≤ 150 ms), GPU-cheap, and off under reduce-motion.
9. Every PR touching a hot path reports before/after numbers from `--perf` in the
   "Performance" section of the PR template.
