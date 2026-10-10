# Performance budget and how to measure it

Oxikube must feel as smooth as Zed. These are the numbers reviewers hold PRs to
(ADR 0013; the frame, dropped-frame, input, memory, idle CPU and notify rows are ADR 0016's
zero-jank budget). Reference machine: Apple M-series laptop, its built-in 120 Hz display; Linux
numbers are measured on a mid-range x86 laptop with an integrated GPU.

## Budgets

| Area | Budget | Scenario |
|---|---|---|
| Frame time | **every frame drawn in ≤ 8.33 ms** (one 120 Hz refresh: `Window::draw` to the end of the content's paint), judged on the maximum (p99 and p95 reported) | every [windowed scenario](#windowed-scenarios-the-zero-jank-budget-in-the-real-window-adr-0016): the pods table, filter, namespaces, detail drawer, tabs and panes, theme, catalog, sidebar, log viewer, terminal |
| Dropped frames | **0** display refreshes missed while a view is driven | the same |
| Input latency | ≤ 1 frame: an input dispatched at a refresh is in that refresh's frame (≤ 8.33 ms to the end of its paint) | keystrokes, scroll events, actions and commands of the windowed scenarios |
| Notifies | ≤ 1 coalesced notify per view per frame | the same |
| Palette | open ≤ 1 frame; filter 2 000 entries ≤ 5 ms | command palette, `:` jump, picker |
| Startup | ≤ 400 ms cold to first interactive frame; catalog before any network; settings + keymap + theme < 30 ms on the main thread | `oxikube` launch with 3 kubeconfigs, 20 contexts ([Startup](#startup-cold-start-to-the-first-interactive-frame)) |
| Cluster open | tab interactive ≤ 200 ms after connect; first table rows ≤ 1 s after feed warm | 2 000-pod cluster |
| Main thread | 0 blocking I/O, process spawn, or lock contention > 1 ms | any |
| Memory | 10 k pods < 400 MB (ADR 0016: the peak of the windowed scenarios whose load is the 10 000-pod cluster alone: `pods-table`, `table-filter`, `namespaces`, `theme`, `sidebar`); idle < 150 MB (2 clusters; ADR 0016: the peak of the windowed `idle` scenario); logs/events ring-buffered | steady state after 10 min |
| CPU idle | < 1 % with two clusters connected and no visible churn (ADR 0016: the windowed `idle` scenario) | laptop on battery |
| Terminal | 60 fps under `yes`/`htop`; resize ≤ 1 frame | local shell + exec |
| Editor | typing latency ≤ 16 ms with validation debounced; 5 MB file opens ≤ 500 ms | manifest editor |
| Agent thread | streaming markdown at 200 tokens/s without dropped frames | ACP panel |

## How to measure

- `cargo xtask perf --windowed <scenario>|--all` runs the real-window scenarios against the
  zero-jank budget (ADR 0016): the app drives its own UI in its window over synthetic clusters and
  writes a per-scenario summary next to the `--perf` JSONL; see
  [Windowed scenarios](#windowed-scenarios-the-zero-jank-budget-in-the-real-window-adr-0016). These
  are the numbers the budget table is about.
- `oxikube --perf` logs per-frame times, feed throughput and `notify` counts to
  `<data dir>/perf/*.jsonl` (`~/.local/share/oxikube/perf` on Linux,
  `~/Library/Application Support/oxikube/perf` on macOS; `$OXIKUBE_DATA_DIR/perf` when that is set,
  `--perf-dir` overrides both) and prints p50/p95/p99 on exit (E01-S14);
  see [Perf harness](#perf-harness-oxikube---perf-and-cargo-xtask-perf).
- `cargo xtask load-pods --count 10000 --churn` seeds the churn scenario on kind (E01-S10); see
  [Load fixture](#load-fixture-cargo-xtask-load-pods) below. `oxikube --perf-table <context>` then
  connects that context, opens its pods table and scrolls it while `--perf` records (E07-S09); see
  [Resource table](#resource-table-10-000-pods-under-churn-e07-s09).
- `cargo xtask perf <scenario>|--all` runs scripted scenarios headless and writes a report; nightly
  CI compares against [`docs/perf/baseline.json`](perf/baseline.json) and fails on > 50 % (p50) or
  > 150 % (p95/p99) regression on Linux (advisory on macOS, whose hosted runners are too noisy),
  > 20 % by default (E01-S14, E01-F542).
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
| `tick` (every second) | `t_ms`, `interval_ms`, `frames_us` (every frame in the interval), `dropped_frames` (frames the recorder lost to ring overflow; not display refreshes, which only the windowed summary counts), `feed_deltas`, `feed_deltas_per_s`, `notifies`, `notifies_per_s`, `max_notifies_per_frame`, `max_view_notifies_per_frame`, `rss_mib`, `peak_rss_mib` (MiB, `null` where the OS has no reader) |
| `summary` (on exit) | `duration_ms`, `frame_count`, `frames` {`count`, `p50`, `p95`, `p99`, `max`} (ms), `dropped_frames`, `feed_deltas`, `feed_deltas_per_s`, `notifies`, `notifies_per_s`, `max_notifies_per_frame`, `max_view_notifies_per_frame`, `rss_mib` {`count`, `p50`, `p95`, `p99`, `max`} (MiB, over the per-tick readings), `peak_rss_mib` |

The JSONL schema is 2 since E01-P587 (`max_view_notifies_per_frame`: the most coalesced notifies one
view received between two frames, which ADR 0016 bounds at one).

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
otherwise. `max_view_notifies_per_frame` is at most 1 by construction since E05-P599:
`notify_coalesced` delivers at the start of the window's next frame (`Window::on_next_frame`), not
on a timer, so a late frame or a 60 Hz display cannot land two notifies on one view before it
draws. A backstop timer delivers only when no frame comes (no window, a window macOS stopped
presenting), and the frame after a backstop delivery leaves the next batch for the frame after it.
A streaming view that can be off screen (the pods table of a background cluster tab) redraws
through `oxikube_runtime::RenderGate`: once notified it is not notified again until it renders, so
it does not collect one notify per refresh while its window draws nothing (before E05-P599 such a
table received 2 between two tab switches in `tabs-panes`).

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

Two rules keep a scenario measuring what the app does (E07-F509, see
[the Linux runner's 340 ms frames](#the-linux-runners-340-ms-frames-509)):

- A view-driving scenario mounts its view the way the app's window does: `oxikube_ui`'s window
  root, then the `--perf` frame hook (`bins/oxikube/src/perf_scenario/window_root.rs`). The root
  gives the view's text the theme's UI family; a sample fails if that text is drawn in a family the
  machine does not have, since GPUI would then walk its fallback stack on every text run.
- Samples run with error backtraces off. `cargo xtask perf` starts each one with
  `RUST_LIB_BACKTRACE=0` (`.cargo/config.toml` sets `RUST_BACKTRACE=1` for every cargo command;
  panic backtraces stay on), and `oxikube --perf-scenario` warns on stderr when they are on. With
  them on, every `anyhow` error built on a hot path walks the stack.

| Scenario | Status | Metrics |
|---|---|---|
| `startup` | measured | the real init order to the main window's first interactive frame (E05-S13, see [Startup](#startup-cold-start-to-the-first-interactive-frame)): `first_frame_ms` (first line of `main` to the end of the update that drew the first frame), `launch_to_first_frame_ms` (process spawn to the first-frame marker on stdout, so exec and dynamic loading are included; timed by xtask), `config_load_ms` (settings + theme + keymap on the main thread), `init_<stage>_ms` (every stage of `oxikube::startup`), `state_db_open_ms` (creating and migrating the SQLite state db, off the UI thread in the app); then `frame_ms` / `draw_ms` (120 idle redraws of the main view: hook time, and wall time of the whole update measured outside GPUI) and `rss_mib` / `peak_rss_mib` (headless resident memory after the redraws, MiB; see [Memory (RSS)](#memory-rss)) |
| `scroll-10k` (alias `table-scroll-10k`) | measured (E07-S09, see [Resource table](#resource-table-10-000-pods-under-churn-e07-s09)) | `first_rows_ms` (table created on a warm feed to the first frame showing all 10 000 pods), `frame_ms` / `draw_ms` scrolling 3 rows a frame while the feed delivers a 10-event batch a frame, `rss_mib` / `peak_rss_mib`; fails unless every batch is counted as feed deltas and `max_notifies_per_frame` ≤ 1 |
| `palette` | measured (E11-S03) | the command palette over 2 000 registered commands (fixture commands in every category, view and selection shape; 1 500 available for a table with a pod selected): `open_ms` (building the view, which classifies the commands and puts the recents first, to the end of the first frame listing them; a fresh process, so it includes the real text system's first glyph shaping), then 120 scripted frames typing `fixture 19 pod` one character a frame (cleared after the last): `frame_ms` / `draw_ms` (the matches of 2 000 candidates, above 512 on the background executor, and the frame showing them), `rss_mib` / `peak_rss_mib`. Local macOS, `release-fast`, median of 5: `open_ms` 20 (the palette's own part is 1.5 ms: `cargo run -p oxikube_palette --features test-support --profile release-fast --example command_palette_bench` reports classify 0.18, open 1.3, keystroke to matches 0.75, frame 0.28, all p50 ms), `frame_ms` p50 0.31 / p95 1.45, `draw_ms` p95 0.72. Baseline: seeded from the nightly run like the others |
| `logs-stream` | measured (E08-S02, see [Log viewer](#log-viewer-streaming-5-000-liness-e08-s02)) | the log view streaming 5 000 lines/s after a 1 000-line tail, 120 scripted frames per mode: `frame_ms` / `draw_ms` (wrap off, following), `paused_*`, `wrap_*`, `wrap_paused_*`, `search_*` / `filter_*` (E08-S03: a regex search highlighting / filtering while it streams), `merged_*`, `merged_wrap_*` (E08-S04: the same lines merged from 10 pods of a Deployment), `rss_mib` / `peak_rss_mib`; fails unless every mode receives the lines at that rate and `max_notifies_per_frame` ≤ 1 |
| `editor-5mb` | unavailable: the `ManifestEditor` view exists (E10-S04; `cargo run -p oxikube_editor --profile release-fast --example typing_bench` measures keystroke-to-frame on a 2k-line manifest on the test platform); the 5 MB scenario needs E10-S11 #153 (large-file and performance hardening) | open time, typing latency |

A scenario whose view is not built yet writes `status: "unavailable"` and a specific `reason` into the JSON report, prints `UNAVAILABLE: <reason>` with the enabling stories, and exits 0. For `editor-5mb`, E05-S11, E10-S04, and E10-S11 unblock implementation. The story that builds that view replaces the stub in `bins/oxikube/src/perf_scenario/mod.rs` with a script driven by `oxikube_runtime::perf::harness::run_frames` and record a baseline in the same PR. The scripted path and the sample schema are covered by a `#[gpui::test]` that drives a fake feed through the same driver (`oxikube_runtime::perf::harness`).

### Baseline and the nightly gate

[`docs/perf/baseline.json`](perf/baseline.json) holds p50/p95/p99 per metric, keyed by OS
(`linux`, `macos`) and scenario. The numbers come from the nightly's own runners
(`ubuntu-latest`, `macos-latest`), because the gate compares a runner with itself; numbers from a
laptop are not comparable with a CI VM.

`--check` fails when any p50/p95/p99 of a baselined metric is more than **+20 %** higher
(`--tolerance`; `--tail-tolerance` sets p95/p99 separately) **and** higher by more than an absolute
noise floor, which depends on the class of the metric (`xtask/src/perf/floors.rs`):

- Frame metrics (`frame_ms`, `draw_ms` and every `<mode>_frame_ms` / `<mode>_draw_ms`) and the small
  startup stages (`config_load_ms`, `state_db_open_ms`, `init_<stage>_ms`): **0.25 ms**
  (`--noise-floor-ms`). It stops microsecond jitter on sub-millisecond metrics from failing the job
  and is far below any budget in the table above. With the real views in (`scroll-10k` 3 to 5 ms,
  `logs-stream` 1.6 to 3.5 ms a frame, the idle redraw 0.4 to 0.9 ms) it is under 10 % of a frame, so
  these metrics gate on the +20 % rule and a frame that doubles fails. When the placeholder app drew
  in 0.01 ms the same floor let a 10x slowdown through (#411).
- Cold-start milestones, one value per launch (`first_frame_ms`, `launch_to_first_frame_ms`,
  `first_rows_ms`, `init_window_ms`): **40 ms** (`--noise-floor-cold-ms`). The same code read 95.7 ms
  and 120.3 ms (+25.7 %) on two `ubuntu-latest` nightlies (runs 37125615040 and 37126183063) and
  66 to 107 ms across five `macos-latest` ones; a 20 % rule with a 0.25 ms floor failed the job on
  that weather. Under +40 ms is noise, and the 400 ms absolute budget still applies.
- `*_mib` metrics: **8 MiB** (`--noise-floor-mib`). Memory has its own floor because the ms floor
  does not apply to it (0.25 MiB would fail on allocator noise) and +20 % of a small RSS is only a
  few MiB. The run-to-run spread of the headless startup scenario is about 0.1 to 0.3 MiB on macOS,
  so 8 MiB sits well above jitter and below 6 % of the 150 MB idle budget.

The nightly passes `--tolerance 0.5 --tail-tolerance 1.5` and repeats a failed check once
(E01-F542). Linux stays within a few percent on the frame metrics (sub-millisecond stage timings
move by up to 0.5 ms), but the hosted macOS runner does not: across nightlies of the same code p50
moved +20 to +90 %, p95/p99 up to +280 %, and even `state_db_open_ms` and `init_window_ms` (no app
code) +65 %. +20 % made the gate fail on weather, and no relative tolerance that still catches
a regression survives that. So the Linux check blocks the nightly, and the macOS check is advisory:
the step may fail without failing the job, the report is still uploaded (`perf-report-macOS`) and
a warning annotation says the check failed. A quiet-machine comparison (`cargo xtask perf --all
--check` locally, 20 %) is the way to confirm a suspected macOS regression; a stable macOS signal
(instructions retired, or a dedicated runner) is the way to gate on it again. The absolute budgets
above are part of every check. The 20 % default is for a quiet machine compared with itself.

The floors were re-tuned in E08-F520 (#411), once `scroll-10k` and `logs-stream` put the frame metrics
in the millisecond range: they now gate against their baselines as well as against the absolute
budgets. A scenario or metric with no baseline is reported as
`MISSING` and does not fail; a scenario that has a baseline but no longer runs does fail.

The nightly `perf` job (ubuntu + macOS) runs `cargo xtask perf --all --check --samples 7`, uploads
`perf-report-<OS>` and, on failure, feeds the `nightly-failure` tracking issue.

Committed numbers (`startup`, median of 7 samples; ms for timings, MiB for memory; re-seeded from
nightly run 37715509006, E08-F520, which runs the real init order), with a local
M-series laptop run for reference (not gated):

| Metric | `linux` (ubuntu-latest) p50 / p99 | `macos` (macos-latest) p50 / p99 | local M5 Max (20 samples) p50 / p99 |
|---|---|---|---|
| `launch_to_first_frame_ms` | 112.7 / 112.7 | 80.4 / 80.4 | 146.2 / 146.2 |
| `first_frame_ms` | 98.6 / 98.6 | 70.1 / 70.1 | 139.5 / 139.5 |
| `config_load_ms` (settings + theme + keymap) | 1.80 / 1.80 | 1.72 / 1.72 | 0.78 / 0.78 |
| `state_db_open_ms` (off the UI thread in the app) | 2.54 / 2.54 | 1.91 / 1.91 | 2.6 / 2.6 |
| `frame_ms` (idle redraw, hook) | 0.71 / 0.77 | 0.44 / 0.90 | 0.037 / 0.044 |
| `draw_ms` (idle redraw, outside) | 0.73 / 0.79 | 0.45 / 0.95 | 0.039 / 0.047 |
| `rss_mib` (headless, after the redraws) | 138.0 / 138.0 | 51.6 / 51.6 | 44.9 / 44.9 |
| `peak_rss_mib` (headless) | 138.0 / 138.0 | 51.6 / 51.6 | 44.9 / 44.9 |

The `init_<stage>_ms` breakdown is baselined too (`docs/perf/baseline.json`). On the macOS runner the
component library's `init` (`init_ui_ms`, the font enumeration) and the platform (`init_assets_ms`)
dominate as on the laptop; on the Linux runner (lavapipe) they are 2-3.5 ms and opening the window with
its first draw (`init_window_ms`, about 102 ms) is the whole cost.

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


## Windowed scenarios: the zero-jank budget in the real window (ADR 0016)

Built in E01-P587. The app, in its real window on the real GPU, drives its own UI through the paths
a user's input takes and is measured against the budget table above. Code:
`crates/platform/oxikube_runtime/src/perf/windowed/` (the pacer, the meter, the summary and its
verdict), `bins/oxikube/src/perf_window/` (the scenarios, the synthetic clusters, the driver),
`xtask/src/perf/windowed/` (the runner and the report).

```
cargo xtask perf --windowed pods-table          # one scenario, 5 runs
cargo xtask perf --windowed --all               # every scenario, 5 runs each (about 40 minutes)
cargo xtask perf --windowed --all --enforce     # fail when a scenario is over budget
cargo build -p oxikube --features perf-window --profile release-fast
target/release-fast/oxikube --perf-scenario-window logs     # one run by hand
```

Keep the window in front for the whole run and the machine otherwise idle: a window that is not
the active one is paced at 30 fps by GPUI (the run is reported as `valid: false`, no measurement),
and macOS stops refreshing a window nobody can see (hidden, minimised, behind a full-screen app on
another Space): the run then stops after 5 s without a refresh and says so.

### How a run works

- **Start-up.** `oxikube --perf-scenario-window <name>` (feature `perf-window`, implies `--perf`)
  starts through the real init order with the embedded default settings (nothing of the user's is
  read or written), an in-memory state db, the app's Tokio runtime and wall clock, and the
  scenario's synthetic clusters (`bins/oxikube/src/perf_window/world/mod.rs`). The window opens at
  1440 x 900 pt. It waits until the window is the active one, then measures the display's refresh
  (the median gap between refreshes while nothing is drawn: 8.33 ms on the reference Mac).
- **Setup** goes through the user's commands: `cluster::Connect`, `resource::OpenList`,
  `resource::Open`, `pod::ViewLogs`, `terminal::New`. Its frames are the `setup` phase, reported and
  not judged.
- **Scripted phases** run one step on every display refresh (`windowed::drive`: GPUI's
  `on_next_frame`, from the display link, before the frame is drawn). A step dispatches what a user
  does at that moment: a scroll event at the middle of the window (`Window::dispatch_event`), a
  keystroke (`Window::dispatch_keystroke`: key bindings, then the focused input), an action
  (`Window::dispatch_action`), a command on the bus, a window or dock resize. It marks the input,
  whose latency then runs to the end of the frame that shows it. A script fails when its input does
  not reach its view (a scroll that does not scroll, a filter that does not filter), so a broken
  script cannot measure an idle window instead.
- **Idle phases** drive nothing: the frames the app draws on its own, and its CPU. Every 250 ms
  they check that the window is still the active one (`activity_checks`, `inactive_checks`), so an
  idle-only scenario (`idle`) is a measurement exactly when its window stayed in front.

| Scenario | Load | Phases |
|---|---|---|
| `pods-table` | 10 000 pods in 8 namespaces, 1 % recycled every 5 s (`load-pods --churn`'s MODIFIED, DELETED, ADDED, in 10 batches over a second) | `scroll` (20 s, a 90 px scroll event every refresh, reversing at either end), `still` (10 s idle) |
| `table-filter` | the same | `type-filter` (15 s: `table::FocusFilter`, then `load-012` typed at 15 keys/s, erased, again) |
| `namespaces` | the same | `switch-namespace` (15 s: `namespace::Select` 4 times a second through the 8 namespaces and all) |
| `detail-drawer` | the same, and a 5 MB ConfigMap with 40 events | `cycle-tabs` (20 s: `resource_detail::ShowTab` 1 to 4, Overview, YAML, Describe, Events, every half second, focus in the drawer) |
| `tabs-panes` | three clusters: 10 000, 3 000 and 3 000 pods, churning; each with its pods table open | `switch-tabs` (10 s, `cluster::NextTab` 4 times a second), `resize-window` (10 s, the window's size swept every refresh), `resize-dock` (10 s, the front cluster's right dock with the detail drawer swept every refresh) |
| `theme` | the pods table under churn | `switch-theme` (15 s: One Light / One Dark every half second through the settings store, the path a `settings.json` hot reload takes) |
| `catalog` | 50 contexts | `type-search` (15 s: `listed-42` typed at 15 keys/s into the catalog's search, erased, again) |
| `sidebar` | 10 000 pods churning, the cluster's first screen (Workloads overview) with the sidebar's count badges | `churn` (30 s, every refresh watched, nothing driven) |
| `logs` | a pod writing 5 000 lines/s (the mixed JSON / plain stream of the headless `logs-stream`) | `stream-json` (10 s, JSON mode, the default), `type-search` (10 s: `logs::Find`, then `slow` typed and erased), `stream-raw` (10 s, `logs::ToggleJsonMode` off) |
| `terminal` | a real shell: `LocalPty` running `/bin/sh` (`SHELL=/bin/sh` from xtask), or `pod::Shell` in a busybox pod on kind | `yes-flood` (until a 50 MB `yes` ends), `redraw-60hz` (8 s: an `awk` script rewriting every row in colour 60 times a second), `resize` (8 s, the window swept every refresh during the redraw) |
| `idle` | two clusters, 1 000 pods each, no churn, both pods tables open | `idle` (30 s): the idle CPU and idle memory (< 150 MB, judged on the run's peak RSS) budgets |

The synthetic clusters are only what a cluster would send (contexts, watches, lists, gets, logs,
describe text); every view, store, service, the session manager and the command bus are the app's.
`cargo xtask perf --windowed` creates the terminal scenario's pod (`oxi-perf-tty-<pid>`, deleted
afterwards) when `--exec-context` (default `kind-oxikube`) answers.

### The summary

Each run writes the `--perf` JSONL as before and `<jsonl stem>.<scenario>.summary.json` next to it
(`--perf-report` to choose; format in `docs/perf/windowed-summary.example.json`, schema 1): per
phase and for all scripted phases together the frames (count, p50, p95, p99, max: `Window::draw` to
the end of the content's paint, deferred overlays such as popovers, menus and dropdowns included,
which the hook marks with a probe painted as the last deferred draw; only the paint of the window's
tooltip, in-window prompt or drag preview comes after it), the
same frames to the end of `present` (`presented_ms`, reported, not judged: GPUI's Metal `present`
waits for a free drawable, about until the next refresh while the window draws on every one, so it
measures the display's pacing; a present that runs long shows as a dropped frame), the frames over
8.33 ms, the dropped refreshes, the refresh gaps, input latency, notifies per frame and the most for
one view, feed deltas, CPU % of one core, RSS and peak RSS at the phase's end; every scripted
frame over budget with its phase and time; the verdict (`failures`, `valid`). It prints one line
per phase and the verdict, for example (`release-fast`, M5 Max, the machine shared with other
builds):

```
oxikube --perf-scenario-window: pods-table scroll (Driven, 20.0 s): 2399 frames: p50 3.42, p95 3.89, p99 4.09, max 5.39 ms; 0 over budget; dropped 0; input p95 3.92, max 5.45 ms; notifies at most 3 per frame, 1 per view; cpu 52.79 %; rss 274.5 MiB (peak 274.5)
oxikube --perf-scenario-window: pods-table: within every ADR 0016 budget
```

`cargo xtask perf --windowed` writes `<target>/perf/windowed-report-<os>.json` (every run's summary,
and per figure the median and the worst run) and prints a table of them. A run that could not
measure (its script failed, the process crashed or ran past 10 minutes) is kept in the report as an
error (`errors`) and the next run goes on; `--enforce` fails on it.

### Baseline

**First baseline: one run per scenario, on a busy machine (E01-P587, 2026-10-09).** `cargo xtask
perf --windowed --all --samples 1`, `release-fast`, Apple M5 Max, built-in 120 Hz display (measured
refresh 8.31 to 8.34 ms), window 1440 x 900 pt at scale 2, in front for the whole run. The machine
was **not** quiet: load average 35 to 50 on 18 cores (other agents building and testing, eight
stray `yes` processes each holding a core). These numbers say where the gaps are; the five quiet
runs per scenario ADR 0016 asks for replace them (the story's verify stage). On the same machine
with less load (2026-10-08, the example summary in `docs/perf/`) `pods-table` met every budget:
2 399 scroll frames, max 5.39 ms, 0 dropped.

| Scenario | Frames | max / p99 / p95 ms | over 8.33 ms | dropped | input max ms | notifies / view / frame | peak RSS MiB | CPU % |
|---|---|---|---|---|---|---|---|---|
| `pods-table` | 1602 | 19.84 / 13.88 / 11.99 | 459 | 1199 | 19.92 | 1 | 275.9 | 52.95 |
| `table-filter` | 526 | 14.67 / 12.08 / 9.84 | 96 | 33 | 13.18 | 1 | 276.7 | 39.96 |
| `namespaces` | 159 | 16.48 / 12.38 / 10.78 | 36 | 10 | 25.84 | 1 | 286.2 | 11.75 |
| `detail-drawer` | 127 | 685.25 / 93.57 / 9.86 | 11 | 99 | 707.51 | 1 | 582.3 | 10.04 |
| `tabs-panes` | 1685 | 15.32 / 11.69 / 7.98 | 71 | 812 | 17.85 | **2** | 362.4 | 44.44 |
| `theme` | 76 | 15.78 / 15.78 / 9.40 | 4 | 2 | 16.20 | 1 | 269.2 | 4.16 |
| `catalog` | 226 | 9.46 / 5.83 / 4.08 | 1 | 1 | 9.71 | 0 | 142.3 | 6.12 |
| `sidebar` | 12 | 3.60 / 3.60 / 3.60 | 0 | 0 | - | 1 | 262.4 | 0.83 |
| `logs` | 1136 | 14.05 / 8.84 / 6.22 | 17 | 17 | 12.82 | 1 | 292.3 | 24.85 |
| `terminal` | 2599 | 8.01 / 5.15 / 3.98 | 0 | 984 | 7.15 | **2** | 267.8 | 42.82 |
| `idle` | 0 | - | 0 | 0 | - | 0 | **180.8** | 0.48 |

Frames are the scripted phases' (`Window::draw` to the end of the content's paint); CPU is % of one
core over the scripted phases (for `idle`, the idle CPU budget's figure). The `idle` run was not
counted as a measurement then (an idle-only scenario had no refresh to check the window on; idle
phases now check it every 250 ms), its 0.48 % is within the 1 % budget. Peak RSS is within 400 MB
(381 MiB) in every 10 000-pod scenario. `idle`'s 180.8 MiB is **over** its 150 MB (143 MiB) idle
budget (the run was judged before that budget was added to the scenario; it is judged now). The drawer's 582 MiB is reported, not judged (the 5 MB object is
more than 10 000 pods); that the 5 MB object and its YAML editor cost about 300 MiB over the pods
table is itself a finding for the quiet profile.

**Over-budget frames and their causes.** Each was placed by its phase and time in the summary
(`over_budget`), against the script's input times and the 5 s churn period, and the drawer's by a
`sample` time profile of the run (Instruments is not installed on the measuring machine; `sample`
is its command-line sampler).

| Scenario | Over-budget frames | Cause |
|---|---|---|
| `detail-drawer` | 1 frame of 685 ms (1 057 ms in a second run) at the first switch to YAML; 93.6 ms at the next switch; the rest 8.3 to 12 ms at tab switches; 99 refreshes dropped around them | The YAML tab makes its text and its editor on the UI thread the first time it is drawn (`detail/yaml/tab.rs` `refresh_yaml`: `yaml_text` serialises the 5 MB object; then `EditorState` takes the 5 MB string: rope build, tree-sitter parse, line wrapping). The profile's main thread is in `ropey::Rope::line_to_byte_idx`, `ts_lexer__do_advance` and `LineWrapper::wrap_lines` under `Window::draw`. Fix: build the text and the editor's buffer off the UI thread and show the editor when it is ready. |
| `namespaces` | 36 frames, 8.4 to 16.5 ms, one at each `namespace::Select` (every 250 ms); input max 25.8 ms | The frame that shows the narrowed or widened row set: the table's visible rows are new rows, so their cells are made, shaped and laid out in that frame. The command reaches the table through the bus a refresh after its dispatch, so its input latency is about two frames (p95 20.5 ms). Placed by timing; which part of the frame dominates is for the quiet profile. |
| `table-filter` | 96 frames, 8.3 to 14.7 ms, at the keystrokes (one every 67 ms) | As for `namespaces`: the frame that shows each key's new row set makes the new visible rows' cells (the filter itself runs in the store's subscription, `set_filter_parts`). Placed by timing; to profile. |
| `theme` | 4 frames, 9.4 to 15.8 ms, at theme switches | The frame after a switch re-renders every view of the window with the new colours (the pods table's visible cells included). The quiet profile (#602) placed the long one at the **first** switch: about 4 ms of it rasterising every visible glyph again, because the new theme's text colours have another glyph dilation. Fixed by the glyph warm-up, see [Theme switch](#theme-switch-glyph-warm-up-e05-p602). |
| `logs` | 17 frames, 8.5 to 14.1 ms, in bursts at 5 s intervals (5.0 s, 10.1 s, 15.0 s, 20.0 s, ...) in every mode | They come with the pods churn (100 pods every 5 s) of the cluster behind the log view, not with the log stream: the churn's store updates and the sidebar's badge redraw land in the frames the log view draws. To profile on the quiet machine. |
| `pods-table` | 459 frames, 8.4 to 19.8 ms; 1 199 refreshes dropped (the scroll drew at 60 Hz) | The scroll's frames drew in 5 ms at p50 but presented at 16.5 ms: `present` waited two refreshes for a drawable, the window composited at half rate. On the less loaded machine the same scroll presented every refresh (0 dropped, max 5.39 ms), so this run's drops and its long tail are the machine's contention, not the table. Re-measure quiet. |
| `tabs-panes` | 71 frames, 8.4 to 15.3 ms (most while resizing the window); 812 refreshes dropped (resize-dock drew at 60 Hz); a view received 2 coalesced notifies in one frame | Resizing lays out every pane at the new size in the frame (the window resize lays out three cluster tabs' visible views); the dock resize presented at half rate as in `pods-table` (contention). The 2 notifies per view: `notify_coalesced` is paced by an 8.33 ms timer, not by frames, so when a frame is late (or the display runs at 60 Hz) one view gets two notifies before it draws. Fix: deliver coalesced notifies once per frame (the window's next frame), not on a timer. Done in E05-P599 (below), which also found that the `switch-tabs` case is a different one: the pods table of a background cluster tab, notified at two refreshes between two drawn frames. |
| `terminal` | none over 8.33 ms; 984 refreshes dropped in `yes-flood` (presented at 60 Hz) and 48 in `resize`; 2 notifies per view per frame in `yes-flood` | The flood's frames drew in 4 ms but presented at 16.5 ms (as `pods-table`: contention, re-measure); the 2 notifies per view are the timer-paced coalescing above, with the PTY reader notifying the terminal view (fixed in E05-P599, below). |
| `catalog` | 1 frame, 9.46 ms, at a keystroke | p95 4.08 ms: a contention outlier; re-measure quiet. |
| `idle` (memory) | none (no scripted frames); peak RSS 180.8 MiB, over the 150 MB (143 MiB) idle budget by about 38 MiB | Two connected clusters of 1 000 pods each with both pods tables open, after setup. Not yet placed: the quiet run's RSS at the idle phase's start and end (steady state against peak) and a heap profile say whether it is the stores, the two tables' cached rows, the fonts and atlas, or allocator slack. |

Fix stories are filed from this report once the quiet runs confirm it (ADR 0016, rule 2).

### Theme switch: glyph warm-up (E05-P602)

GPUI keys a glyph in its atlas by its *dilation* too: on macOS the stroke thickening CoreGraphics
applies depends on the luminance of the text colour (five levels), so One Dark's light text and
One Light's dark text are different glyphs. The first frame after switching to a theme whose text
sits at other levels missed the atlas for every glyph on screen and rasterised each one with
CoreText (`CTFontDrawGlyphs`), one by one, inside `Window::paint_glyph`: about 4 ms of a 9 ms
frame in the #587 baseline's profile, every run's longest frame.

`oxikube_theme::glyph_warm` moves that work out of the frame without changing what is drawn. The
app is built on a platform wrapper whose text system is the platform's, decorated: it records
every glyph GPUI rasterises, and a worker thread rasterises the same glyphs at the levels a switch
to any installed theme would draw them at (a `DilationPlan`: every colour slot of the active theme
paired with the same slot of each installed theme, remade when a theme is selected or the
registry changes). It uses the same platform calls with the same parameters, so the bitmaps are
the ones the frame would have made; it runs only after the text system has been left alone for
2 ms (between frames: the platform shapes text under a write lock a rasterisation in flight would
hold off), and keeps at most 8 MiB. When the switch draws, GPUI misses the atlas as before and the
prepared bounds and bitmaps answer: the frame only uploads them. Nothing is warmed by the scenario
or before it: the warm-up is the app's, for every user, from the first glyph it draws.

`cargo run --release -p oxikube_theme --example glyph_warm` measures it on CoreText: a screen of
the pods table (60 rows, 117 distinct glyphs in the system UI font at 13 px, scale 2), drawn at
One Dark's text level, then the One Light frame's glyph work (bounds and bitmap of every glyph;
the atlas upload is the same either way and not included). Three runs, M5 Max, machine shared with
other builds (load average about 100):

| | run 1 | run 2 | run 3 |
|---|---|---|---|
| switch frame, no warm-up (rasterised in the frame) | 3.87 ms | 2.10 ms | 3.03 ms |
| switch frame, warmed (117 served, 0 rasterised) | 0.028 ms | 0.046 ms | 0.027 ms |
| the plan, remade on the UI thread at each switch (median of 200) | 4.6 µs | 4.5 µs | 3.4 µs |
| the worker's warm-up, off the frame (117 glyphs, 33 KiB kept) | 0.83 ms | 0.96 ms | 0.84 ms |

(The worker's figure is its second rasterisation of those glyphs in the process, after the
no-warm-up frame's; CoreText's own caches make it lower than a first one would be.) The example
also checks that what is drawn does not change: every prepared bitmap is byte for byte the one
CoreText makes in the frame (117 of 117).

**The windowed `theme` scenario**, 5 valid runs of each build, one after the other (the same
scenario, themes, switch rate and table under churn; nothing excluded): `release-fast` with
`--features perf-window`, run as `oxikube --perf-scenario-window theme` (what `cargo xtask perf
--windowed theme` runs), M5 Max, 120 Hz built-in display, window in front, desktop idle for 20 s
before each run, no cargo build running, load average 29 to 33 (other agents' processes).
`before` is `origin/main` at `ea1fce2d`, `after` this story. Frames are the scripted
`switch-theme` phase's; RSS is the phase's peak; CPU is % of one core.

| run | frames | max / p99 / p95 ms | over 8.33 ms | dropped | input max ms | notifies / view / frame | peak RSS MiB | CPU % |
|---|---|---|---|---|---|---|---|---|
| before 1 | 77 | 9.21 / 9.21 / 4.02 | 1 | 1 | 9.71 | 1 | 274.2 | 3.34 |
| before 2 | 76 | 9.11 / 9.11 / 3.82 | 1 | 0 | 9.37 | 1 | 274.4 | 3.23 |
| before 3 | 75 | 8.77 / 8.77 / 3.98 | 1 | 0 | 9.03 | 1 | 274.7 | 3.24 |
| before 4 | 76 | 8.82 / 8.82 / 3.83 | 1 | 0 | 9.07 | 1 | 275.8 | 3.23 |
| before 5 | 77 | 8.92 / 8.92 / 3.85 | 1 | 0 | 9.16 | 1 | 276.4 | 3.28 |
| **after 1** | 76 | 4.54 / 4.54 / 3.98 | 0 | 0 | 4.85 | 1 | 274.2 | 3.32 |
| **after 2** | 77 | 4.00 / 4.00 / 3.87 | 0 | 0 | 4.19 | 1 | 276.4 | 3.29 |
| **after 3** | 77 | 4.29 / 4.29 / 3.74 | 0 | 0 | 4.22 | 1 | 277.4 | 3.28 |
| **after 4** | 76 | 4.10 / 4.10 / 3.87 | 0 | 0 | 4.28 | 1 | 275.8 | 3.28 |
| **after 5** | 75 | 4.29 / 4.29 / 3.89 | 0 | 0 | 4.31 | 1 | 276.7 | 3.26 |

Every `before` run is over budget with exactly one frame, the first switch (8.8 to 9.2 ms, its
input 9.0 to 9.7 ms); every `after` run is within every ADR 0016 budget, its longest frame 4.0 to
4.5 ms and its longest input 4.2 to 4.9 ms, the same as the later switches. Memory and CPU are
unchanged (the prepared glyphs of a screen are tens of KiB). The first interactive frame of the
same runs: 314 to 410 ms before, 328 to 364 ms after (the wrapper and the warmer cost nothing
measurable at startup). The verify stage re-measures on a quiet machine.

### E05-P599: coalesced notifies paced by frames

`notify_coalesced` delivered on an 8.33 ms timer started by the first event, so when a frame came
late a view could be notified twice before it drew. It now delivers at the start of the window's
next frame (`Window::on_next_frame`, before the draw), with a backstop timer only for windows that
draw no frame (see "max_notifies_per_frame" above). The `tabs-panes` runs then showed a second,
unrelated case in `switch-tabs`, where the window draws only about 10 frames a second: the pods
table of a **background** cluster tab was notified at two refreshes between two drawn frames
(instrumented run: 3 to 4 deliveries since the last drawn frame, the same `ResourceTable` twice).
Its feed redraw now goes through `oxikube_runtime::RenderGate`: a view that has not rendered its
last notify is not notified again (on screen the window is already drawing it; off screen it is
rendered fresh when shown).

Measured with `oxikube --perf-scenario-window`, `release-fast` + `perf-window`, Apple M5 Max, 120 Hz
built-in display, window 1440 x 900 pt, desktop lock held, each run after at least 20 s without
input. The machine was **not** quiet: load average 17 to 41 on 18 cores (other agents building),
eight stray `yes` processes not ours each holding most of a core, and the user returned to the
machine during the session (runs that lost the window or the display are discarded, as the harness
says). The verify stage re-measures on a quiet machine.

`tabs-panes`, valid runs (`before` = `origin/main` `ea1fce2d`; `after` = this story):

| Build | Runs | per view per frame (each run) | scripted max / p99 / p95 ms | dropped | notifies | input max ms | CPU % | peak RSS MiB |
|---|---|---|---|---|---|---|---|---|
| before | 6 | 2, 1, 1, 2, 1, 2 (each 2 in `switch-tabs`) | 6.82-9.89 / 4.34-5.96 / 4.09-5.39 | 1-44 | 253 | 13.4-15.0 | 42.1-50.9 | 352-366 |
| frame-paced only | 5 | 2, 1, 1, 1, 2 (each 2 in `switch-tabs`) | 6.96-7.46 / 4.46-5.23 / 4.20-4.61 | 2-23 | 253 | 13.2-14.7 | 41.7-44.0 | 364-366 |
| after | 5 | **1, 1, 1, 1, 1** | 7.49-8.07 / 5.15-5.59 / 4.66-4.97 | 226-298 | 146-149 | 13.3-15.1 | 42.3-43.6 | 355-369 |

The `after` runs' dropped refreshes are in `resize-window` (49-113) and `resize-dock` (168-189),
whose frames drew at p95 4.3-5.2 ms (before: 3.7-5.7 ms over its six runs) but presented at p95
20-24 ms in `resize-dock`: the window composited at half rate, as in the E01-P587 baseline under
contention. They were taken in
the same minutes as two discarded runs whose calibration measured the display refreshing at 24-25
Hz, so the display pipeline itself was throttled then; the change removes work (107 fewer notifies
per run) and adds none to a resize frame. Re-measure on the quiet machine.

`terminal`, valid runs with the shell in a busybox pod on `kind-oxikube` (`--perf-exec`, what
`cargo xtask perf --windowed terminal` does when that context answers), alternating `before` and
`after`, load average 47 to 73 on 18 cores (the same stray `yes` processes, other agents' test
runs):

| Build | Runs | per view per frame (each run) | `yes-flood` s | scripted max / p99 / p95 ms | dropped | notifies | input max ms | CPU % | peak RSS MiB |
|---|---|---|---|---|---|---|---|---|---|
| before | 5 | 2, 11, 2, 2, 2 (each in `yes-flood`) | 15.9-111.4 | 4.90-10.65 / 3.40-6.31 / 2.85-5.26 | 175-1102 | 1745-3160 | 4.93-10.69 | 17.6-41.7 | 220-267 |
| after | 5 | **1, 1, 1, 1, 1** | 12.0-58.8 | 4.90-8.99 / 3.20-6.20 / 2.64-5.16 | 213-1755 | 1961-2796 | 4.93-9.11 | 27.6-43.7 | 251-289 |

One more `after` attempt, the first, started at load 67 and is not a measurement: its flood did not
end within the 120 s deadline. The `yes-flood` time is the machine's: the first runs, at load 62 to
69, took longest (97.0 and 111.4 s before, 58.8 s after); the other seven took 12 to 20 s. The frames
over 8.33 ms (at most 4 per run, in both builds) are all in `resize`; the dropped refreshes are in
`yes-flood` and `resize` in both builds, and peak RSS is taken at the end of `resize`. Re-measure on
the quiet machine.

The first attempts at `terminal` ran the shell on this machine (no `--perf-exec`) and none
finished the flood within the deadline: the local PTY itself is that slow here, `script -q
/dev/null sh -c 'yes | head -c 5242880'` (a tenth of the flood, nothing of Oxikube involved) takes
108 s. Over the 88-123 s of `yes-flood` they did cover, the most notifies one view received between
two frames was 2, 5, 2, 2 and 5 before and 1, 1 and 1 after. The pacing of the terminal's notifies
is also pinned by `oxikube_terminal`'s `tests/state/frame_paced.rs`: a flood with frames one to
three refreshes late gives at most one notify per frame (the timer gave up to 4).

### E05-P600: UI-only commands land in the input's frame

`namespace::Select` and the cluster tab commands (`cluster::NextTab`, the `ctrl-tab` command, and
`Select`, `SwitchTab`, `PreviousTab`, `CloseTab`) went through the command bus on a Tokio task and
came back to the UI thread in a later update, so the frame drawn right after the input could not
show them: input latency was one refresh plus the frame (12-13 ms at p50). They are now **immediate
commands** (`CommandRegistry::register_immediate`, see `oxikube_app::command_bus::immediate`):
`ClusterCommandRunner` runs them with
`CommandBus::dispatch_now` inside the dispatching update, applies the cluster tabs' queue at once,
and hands the session updates they made to the views in that update (`SessionEcho`). The pods
table keeps the held rows of the new scope and relayouts its columns in the same update (one
namespace hides the Namespace column); the store's reseed snapshot reconciles the rows in a later
frame. The I/O that completes `namespace::Select` (remembering the selection in `StatePort`) runs
off the UI thread. The keys (`ctrl-tab`, `cmd-1..9`), a hotbar click and the namespace selector's
digits apply in the same way. The scripts are unchanged: they still mark the input and dispatch
through `ClusterCommandRunner::run`.

Measured with `cargo xtask perf --windowed <scenario> --samples 1 --bin <binary>`, `release-fast` +
`perf-window`, Apple M5 Max, 120 Hz built-in display, window 1440 x 900 pt, desktop lock held, each
run after at least 20 s without input, `before` (`origin/main` `62115b39`) and `after` alternating
run by run. The machine was **not** quiet: load average 7 to 22 on 18 cores (other agents building
and testing; no cargo or rustc process of this story ran during the runs). An earlier `tabs-panes`
batch at load 40 to 50 had the same latencies with a longer tail (`switch-tabs` input max up to 13.8
ms, from switch frames of 11 to 13.6 ms that the `before` build also drew under that load). The
verify stage re-measures on a quiet machine.

`namespaces` (`namespace::Select` four times a second under 10 000-pod churn), 5 valid runs each:

| Build | `switch-namespace` input p50 / p95 / max ms | frames | frame max / p99 / p95 ms | dropped | notifies per view per frame | CPU % | peak RSS MiB |
|---|---|---|---|---|---|---|---|
| before | 12.65-13.12 / 13.41-14.23 / **14.77-16.55** | 94-98 | 5.58-7.24 / 5.58-7.24 / 4.86-5.53 | 0 | 1 | 4.35-4.98 | 195-198 |
| after | 2.34-2.62 / 4.00-4.61 / **4.49-5.55** | 153-157 | 5.24-6.18 / 4.79-5.99 / 4.46-5.13 | 0 | 1 | 5.53-6.42 | 196-199 |

The after build draws about one more frame per selection (153-157 against 94-98 in 15 s, 60
selections): the input's own frame, which shows the held rows of the new scope (or the loading
state when the table held none of them), and then the frame of the store's snapshot. That frame is
the price of showing the input at once and costs about 1.2 % of a core at four selections a second.
A first `after` build that rescoped the rows but left the columns to the session stream's later
update had a frame of 8.4 to 16 ms after most selections (the column change re-laid every visible
cell a frame after the rows had); the column relayout now happens in the input's frame too.

`tabs-panes` (three clusters; `switch-tabs`: `cluster::NextTab` four times a second), 5 valid runs
each:

| Build | `switch-tabs` input p50 / p95 / max ms | `switch-tabs` frame max ms | scripted frame max / p99 / p95 ms | scripted dropped | scripted input max ms | CPU % | peak RSS MiB |
|---|---|---|---|---|---|---|---|
| before | 11.78-12.34 / 12.80-13.18 / **12.94-13.53** | 4.28-4.57 | 5.82-8.24 / 4.11-4.29 / 3.87-4.03 | 16-76 | 12.94-13.53 | 39.85-41.17 | 211-215 |
| after | 3.65-3.72 / 4.11-4.49 / **4.19-5.80** | 4.13-5.65 | 5.83-8.69 / 4.09-5.34 / 3.84-4.96 | 11-78 | 4.33-6.12 | 39.63-44.10 | 214-216 |

`switch-tabs` dropped no refresh in either build. The dropped refreshes, and the one frame over
8.33 ms (8.69 ms, `after` run 5), are in `resize-window` in both builds (window resize under
contention, unchanged by this story).

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

## Local terminal backend (E09-S02)

`cargo run --release -p oxikube_terminal --example local_pty_bench [-- <shell>]` measures the backend
alone (no grid, no painting yet). The epic budget is a shell tab with the cluster environment in
< 150 ms; opening is `LocalPty::spawn` (off the UI thread) plus whatever the shell itself needs to
print its first prompt.

Reference machine (macOS, release build, 30 opens each, `/bin/sh`; `/bin/zsh` is the author's
interactive zsh with its own rc files):

| | `spawn` p50 / p95 | open to first output p50 / p95 |
|---|---|---|
| `/bin/sh`, plain | 2.2 / 2.4 ms | 9.1 / 11.0 ms |
| `/bin/sh`, cluster env | 2.6 / 4.7 ms | 9.9 / 15.4 ms |
| `/bin/zsh`, cluster env | 3.3 / 6.4 ms | 181 / 286 ms (the user's rc files, not the backend) |

Cutting and writing the merged kubeconfig costs about 0.3 ms. Read throughput under `yes` is
about 110 MiB/s through the bounded queue (32 chunks of at most 16 KiB; macOS returns about 1 KiB
per read), with one allocation per read and none per byte; the grid coalesces chunks to frame
cadence. A login shell (`terminal.shell_args: ["-l"]`) re-reads the profile and costs the shell's own
start time, which is why it is off by default.

## Terminal element (E09-S05)

`cargo run --profile release-fast -p oxikube_terminal --example element_bench` paints the element
headless (the host's real text system, no GPU time) at 80 x 24 and 240 x 60, one wave of output per
frame. The *frame* is the tick firing the coalesced notify plus the whole window's layout, prepaint
and paint, every reshape included; a *repaint* is a frame with nothing new (hover, focus).

Reference machine (macOS, Apple silicon, Menlo 13 px, 300 frames after 30 warm-up):

| grid | workload | frame p50 / p95 / max (ms) | repaint p50 (ms) | allocs/frame | row cache hits | runs shaped/frame |
|---|---|---|---|---|---|---|
| 80x24 | idle | - | 0.016 | 0 | 100 % | 0 |
| 80x24 | `yes` | 0.025 / 0.049 / 0.072 | 0.022 | 79 | 100 % | 0 |
| 80x24 | `ls --color` (8 lines/frame) | 0.314 / 0.464 / 1.283 | 0.108 | 303 | 83 % | 32 |
| 80x24 | `htop` (every row redrawn) | 0.547 / 0.712 / 1.162 | 0.124 | 608 | 50 % | 72 |
| 240x60 | idle | - | 0.114 | 0 | 100 % | 0 |
| 240x60 | `yes` | 0.134 / 0.178 / 0.235 | 0.132 | 79 | 100 % | 0 |
| 240x60 | `ls --color` | 0.526 / 0.639 / 1.019 | 0.332 | 303 | 93 % | 32 |
| 240x60 | `htop` | 1.555 / 1.979 / 4.049 | 0.405 | 1 400 | 50 % | 180 |

What keeps it there:

- **Rows are cached by content, not position.** A row's laid-out spans and shaped runs are keyed by
  a hash of its cells, so a scrolling log shapes only the new lines, `yes` shapes nothing after the
  first frame, identical rows (blank lines) share one entry, and a repaint shapes nothing. The
  `htop` hit rate is 50 % because every row is new each frame and the bench then repaints once.
- **Runs, not cells.** Adjacent cells of one style are one shaped run (blanks inside it included),
  with every glyph forced to the cell width so a run cannot drift off the grid; adjacent equal
  backgrounds are one quad.
- **Allocations follow what changed.** A repaint allocates only GPUI's own per-frame bookkeeping
  (about 70 to 80); each shaped run costs about 8 more (the text and GPUI's line layout). The
  snapshot, palette, row buffers and spare rows are reused frame to frame.
- **Nothing waits on the grid.** The snapshot uses `try_lock`; a busy grid repaints the previous
  frame and asks for another. Resizes go to the grid at once (the frame shows the new size) and
  reach the process coalesced.

These are headless numbers: compare them with the same machine, not with the 8 ms budget (which
includes GPU time and present). The windowed check is `cargo run -p oxikube_terminal --example
terminal_preview` (a live `top`).

## Terminal input (E09-S06)

`cargo run --release -p oxikube_terminal --example input_bench` times the mapping layer (Apple
silicon, release): `to_esc_str` 2 to 5 ns per keystroke (arrow, ctrl-c, ctrl-alt-shift-f5, alt-b,
and plain text, which maps to nothing), an SGR mouse report 7 ns, a one-line bracketed paste 13 ns.
The mapping hands out `&'static str`s from tables, so a keypress allocates nothing:
`tests/no_alloc.rs` runs it under a counting global allocator and asserts zero allocations for
every key family, modifier and mode, and for mouse reports. The key goes to the writer task through
an unbounded channel (`TerminalState::input`), never waiting on the grid lock beyond one parse
slice. Input to pixel is the PTY round trip plus one frame: the `#[gpui::test]`
`an_echo_is_painted_within_one_frame_of_the_key` sends a key to an echoing fake backend and checks
the echo is in the frame drawn one frame interval later. Settings read per keystroke
(`terminal.option_as_meta`) are read by reference, not cloned.

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
8.3 ms, to the end and back, until the session ends. Code: `bins/oxikube/src/perf_table/mod.rs`.
It logs each step and how long the table took to list. `--perf-also <context>` (repeatable)
connects further contexts first, each with its pods table open and left still, for the
several-clusters idle measurement ([Idle CPU](#idle-cpu-under-the-budget-e07-f512)).

Headless, on the fake feed generator (the CI regression gate, no cluster):

```
cargo xtask perf scroll-10k            # alias: table-scroll-10k
```

The scenario (`bins/oxikube/src/perf_scenario/scroll_10k.rs`) runs the real store, store runtime
and `ResourceTable` on testkit fakes: 10 000 pods listed into a warm store, then 120 scripted frames
(1 s at 120 Hz; each draws twice, once for the feed's coalesced notify and once for the scroll, so
240 frames are measured) scrolling 3 rows each while the feed delivers one 10-event batch per frame
(6 modifies, 2 deletes, 2 creates: 1 200 events/s, about twenty times the load-pods churn). It
fails unless every batch is counted as feed deltas, no frame absorbed more than one coalesced
notify and the table's text is drawn in a family the machine has (E07-F509). Budgets checked by
`cargo xtask perf` on every OS: `first_rows_ms` < 1 000 and `frame_ms` p95 ≤ 18.2 ms (55 fps); being
headless they are lower bounds of the windowed figures. Until E07-F509 the frame budget was macOS
only, because the Linux runner read about 340 ms a frame; that was the scenario, not the runner
([below](#the-linux-runners-340-ms-frames-509)).

### Numbers

Reference machine: Apple M5 Max (18 cores), macOS, built-in display, `release-fast`, story branch
`story/E07-S09-perf-tuning` on `a1d3f1c`; the shared kind cluster (one node, v1.37) with 10 000
unscheduled (`Pending`) load pods in 8 namespaces and `--churn` running; other builds running on
the machine (load average 4 to 26), so treat single digits of a percent as noise.

| Run | Frames | p50 | p95 | p99 | max | Other |
|---|---|---|---|---|---|---|
| windowed, `--perf-table`, 60 s, scrolling 3 rows / 8.3 ms | 6 019 in 60.3 s (100 fps, paced by the scroll timer) | 3.42 ms | 4.00 ms | 4.26 ms | 7.86 ms | 9 931 pods listed 258 ms after the table opened (a cold list from the API server); feed 11 430 deltas; 691 notifies, at most 3 per frame (table, sidebar badges, overview tiles); dropped 0 |
| windowed, table still (`--perf-scroll 0`), 25 s | 293 | 3.96 ms | 4.64 ms | 7.22 ms | 8.20 ms | redraws only for churn and ages |
| headless `scroll-10k`, nightly `macos-latest` (run 37492117478, the baseline until E01-F542; a later run of the same code measured 4.04 / 8.58 ms, p50 / p95, and 43 ms to first rows, so the baseline was re-seeded from run 37647964174: shared-runner variance, not a regression) | 240 per launch | 3.32 ms | 4.37 ms | 5.03 ms | 5.52 ms | `first_rows_ms` 26.4 (p95 30.7 across launches); RSS 176 MiB |
| headless `scroll-10k`, nightly `ubuntu-latest` (same run, lavapipe) | 240 per launch | 338 ms | 343 ms | 348 ms | 370 ms | `first_rows_ms` 466 ms; RSS 269 MiB; the scenario's font fallback with error backtraces on, not the runner (#509, [below](#the-linux-runners-340-ms-frames-509)) |
| headless `scroll-10k`, `ubuntu-latest` after E07-F509 (run 37694476864, the committed baseline) | 240 per launch | 3.17 ms | 3.43 ms | 3.63 ms | 3.76 ms | `first_rows_ms` 88 ms; RSS 272 MiB |
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

Log ingest (E08-S01, `cargo bench -p oxikube_app --bench log_ingest`, the service's own cost over a synthetic
always-ready port of 100-byte lines): 3 000 000 lines (10 minutes at 5 000 lines/s) into a 50 000-line ring in
about 225 ms, 13 M lines/s, 2 000-line batches (about 150 µs to build and commit each, an upper bound of the
time the session's lock is held); copying a 60-row viewport out under the lock costs 0.4 to 6 µs; RSS stays at
the ring's bound (17 MB with a 5 000 to 50 000-line ring) however many lines were read.

Main thread (macOS `sample`, 8 s of the windowed run while scrolling under churn): 55 % idle, 42 %
in `Window::draw`, of which about half is laying out the visible rows (gpui-component's per-row
horizontal virtual list and taffy); the cell path (`TextCell`, `CellCache`, the provider) is about
6 % of the draw and applying the store's deltas under 0.1 %. No sample waits on a mutex or condvar
on the main thread: no lock contention.

### The Linux runner's 340 ms frames (#509)

Until E07-F509 the nightly's `scroll-10k` frames took about 330 ms on `ubuntu-latest` (Mesa
lavapipe) against 3 to 4 ms on `macos-latest`, and `logs-stream` about 17 ms. It was not the
software renderer, and not shaping or rasterising new glyphs (the issue's two suspects). Probe runs
on the runner (4 vCPU AMD EPYC, Mesa 25.2 llvmpipe, fonts: DejaVu, Liberation, Lato, Noto Color
Emoji):

| `scroll-10k` on `ubuntu-latest` (run 37686505840, `release-fast`) | `frame_ms` p50 / p95 | `first_rows_ms` |
|---|---|---|
| `oxikube --perf-scenario scroll-10k` run directly: 10, 40, 120 scripted frames | 5.8 / 6.4, 6.1 / 6.5, 6.6 / 7.0 ms | 138, 125, 128 ms |
| the same binary through `cargo xtask perf scroll-10k --samples 1`: 10, 120 frames | 268 / 275, 267 / 277 ms | 390, 402 ms |

The difference was the environment. `.cargo/config.toml` sets `RUST_BACKTRACE=1` for every cargo
command, so `cargo xtask` ran under it and the sample process inherited it. With it set, every
`anyhow::Error` captures a stack trace when it is built. An Ubuntu 24.04 container with lavapipe
reproduces it with the binary alone: 5.9 ms p50 with `RUST_BACKTRACE=0`, 288 ms with `=1`.

What built errors on every frame: the scenarios drew their view under a bare window root rather
than the app's, so its text asked for GPUI's default family, `.SystemUIFont`. GPUI maps that to
IBM Plex Sans on Linux, which the runner does not have, so `TextSystem::resolve_font` went through
its fallback stack (`.ZedMono`, `.ZedSans`, Helvetica, Segoe UI, Ubuntu, Adwaita Sans, Cantarell,
Noto Sans) to DejaVu Sans on every text run. GPUI caches each miss, but as an error that
`TextSystem::font_id` re-creates with `anyhow!` on every hit (gpui-pre 0.3.7), so every text run
of every frame built one error per missed family. Even without backtraces, `perf record` on the
runner (a direct run) has `TextSystem::font_id` at the top of the self time and building and
formatting `anyhow` errors close behind, with lavapipe at 2 % of the samples. macOS resolves
`.SystemUIFont` itself, so it never missed.

The app does not take this path: its window is `oxikube_ui`'s root (gpui-component's `Root`), which
gives the content the theme's UI family, and gpui-component has already named the installed family
`.SystemUIFont` lands on (DejaVu Sans on the runner). No Linux desktop with a GPU was at hand.
Instead, the windowed app (`oxikube --perf --perf-table`, a product `release-fast` build,
3 000 load pods under churn, an Ubuntu 24.04 container on xvfb + lavapipe, on a laptop at load
average 30 to 40) drew 18 to 44 ms p50 frames with backtraces on and off alike. So there is no error
on its hot path; what remains is software rendering and presenting the whole window. A product bug
on Linux it is not.

What changed (E07-F509):

- `perf_scenario::window_root` mounts the `scroll-10k` and `logs-stream` views as the app's window
  does and fails a sample whose text is drawn in a family the machine does not have (unit tests on a
  fake machine with only DejaVu installed: mounted like the app, the view is drawn in DejaVu Sans;
  outside the root it asks for `.SystemUIFont` and the check fails).
- `cargo xtask perf` runs every sample with `RUST_LIB_BACKTRACE=0`, and `oxikube --perf-scenario`
  warns when error backtraces are on.
- The frame budgets (`scroll-10k` 55 fps, `logs-stream` 8 ms p95) are checked on Linux too.
- The `scroll-10k` baselines were re-seeded from the story branch's runs. The view now sits in
  the real root, which costs every frame what it costs the app (window border, overlay layers, rem
  size): on the M5 Max under the same load, `frame_ms` p50 3.3 → 3.7 ms for the table and +0.1 to
  0.2 ms for the log view.

| Headless, the nightly's perf job (median of 7 launches) | before (run 37672159958, the last green nightly) | after (run 37694476864) |
|---|---|---|
| `ubuntu-latest` `scroll-10k` `frame_ms` p50 / p95 | 329 / 337 ms | 3.17 / 3.43 ms |
| `ubuntu-latest` `scroll-10k` `first_rows_ms` | 468 ms | 88 ms |
| `ubuntu-latest` `logs-stream` `frame_ms` p50 / p95 | 16.6 / 19.6 ms | 1.61 / 2.63 ms |
| `ubuntu-latest` `logs-stream` slowest mode p95 (`filter_frame_ms`) | 25.6 ms | 3.39 ms |
| `macos-latest` `scroll-10k` `frame_ms` p50 / p95 | 3.78 / 5.83 ms (3.8 to 4.5 p50 across four runs) | 5.13 / 9.68 ms (attempt 2; attempt 1 read 7.27 / 13.3 with the unchanged start-up stages 40 to 90 % slow too) |
| `macos-latest` `scroll-10k` `first_rows_ms` | 36.6 ms (36 to 48) | 44.3 ms |

The macOS runner is slower and far noisier than the laptop (E01-F542 measured the same code moving
+20 to +90 % at p50 across nightlies), so its share of the root's cost reads larger there; the
baseline was seeded from the attempt whose start-up stages matched earlier runs. Its `logs-stream`
p95s sit around the 8 ms budget on that runner before and after this change (run 37656531284 read
8.9 ms for `filter_frame_ms` on the old code), which is a runner limit, not this change.

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

### Idle CPU: under the budget (E07-F512)

`--perf` has no CPU metric, so idle CPU is the process's CPU time from `ps -o time=` over a 45 to
60 s window, started once the app has settled (10 s after launch, the table already listed), next to
the redraw count from the `--perf` JSONL (`tick.frames_us`, frames per second after the list
landed). Before this story (#512) the budget was only just met: idle app, no cluster 0.7 to 0.9 %,
about 2 frames/s; connected, pods table open and still, 10 000 pods, 0.93 to 1.18 %, about 1.5
frames/s.

**What drove the redraws** (`oxikube_workspace::window::background`, `oxikube_resources_ui::table`):

1. *The idle app, no cluster: a caret.* The catalog home focuses its search field when it opens,
   and a focused `Input` of the component library blinks its caret from a 500 ms timer that
   notifies the field, which redraws the window, for as long as the field has the focus. The timer
   does not look at the window: a window that lost the key (another app in front, this one still
   visible beside it) kept redrawing twice a second for a caret nobody sees. An inactive window now
   parks its focus (`background::follow`, installed by `MainView`) and gives it back to the same
   element on activation, unless something else took it meanwhile; a blurred field stops its
   timer. The caret of the window you are using still blinks, which is a visible change, and stops
   when the focus leaves the field (opening a cluster tab moves it to the table).
2. *A still table: the age tick.* `ResourceTable` notified itself every second while shown so ages
   would move. It now asks the `CellCache` whether any cell the last frame drew reads differently
   at the current time (re-reading about one screen of cells, no redraw) and notifies only then.
   Most ages are days old and change once a day (kubectl's format: `30d`), so a table of them
   draws nothing; a pod a few minutes old still redraws each second its seconds show. Covered by
   `table::tests::ages` and `CellCache::ages_moved`.

**Numbers** (Apple M5 Max, `release-fast`, `kind-oxikube`; load average 35 to 57 from other
builds, so a tenth of a percent is noise; 10 000 `Pending` load pods in my own namespace, no churn,
the kubeconfig context scoped to it so other agents' pods are not listed;
`--perf-table kind-oxikube --perf-scroll 0`, five runs each, alternating, a pause between runs):

| Build | CPU, one cluster | Redraws, one cluster |
|---|---|---|
| before (`ef05d296`) | 1.38, 1.33, 1.22, 1.33, 1.18 % (mean 1.29 %) | 0.95 to 1.10 frames/s |
| after | 0.64, 0.56, 0.62, 0.73, 0.64 % (mean 0.64 %) | 0.00 to 0.11 frames/s (the strays are a conditions or age change) |

Two clusters (the budget's scenario): `oxikube --perf --perf-duration 75 --perf-table kind-oxikube
--perf-also kind-oxikube-b --perf-scroll 0` connects both contexts (the same kind cluster under two
context names, each scoped to the 10 000-pod namespace, so 20 000 pods are listed) and opens both
pods tables. Three runs after the list landed: **0.13, 0.27 and 0.45 % CPU, 0.00 to 0.04
frames/s**, which is under the 1 % budget. (RSS reads 875 MiB with two 10 000-pod clusters, over
the memory budget: [#508](https://github.com/karan-vk/Oxikube/issues/508).)

The scrolling hot path is unchanged (the only per-frame difference is reading the clock through
`ResourceTable::now`): `cargo xtask perf scroll-10k --samples 3` on this branch, headless, same
loaded machine: `frame_ms` p50 3.58, p95 7.75 ms, first rows 70 ms, at most 1 notify per frame.

Measuring notes: a window that is not visible (covered, another Space) draws nothing, and its CPU
reads 0.1 to 0.3 % whatever the code does, so compare runs whose `--perf` JSONL shows frames in the
first seconds, and compare the redraw count, which does not depend on the machine's load. The
detail drawer is no longer an exception ([E07-F566](#the-detail-age-tick-e07-f566), below).

### The detail age tick (E07-F566)

`DetailView` (the drawer, or pinned as a tab) notified itself every second while shown, so an open
detail of a still object cost one redraw a second. Its ages are drawn by three views: the header's
`age` chip (every tab), the Overview's condition rows and the Events tab's rows. The timer still
fires once a second, but `detail::ages` now re-reads the ages the active tab draws, formatted at the
time of the last frame (`drawn_at`, set by `render`) and now, and notifies only when one reads
differently (`DetailView::ages_moved`; the same re-read-and-compare as the table's
`CellCache::ages_moved`). An object days old changes once a day (`30d`), so a still detail draws
nothing; a young object, or a condition or event a few minutes old on the tab that shows it, still
ticks each second its seconds show. A hidden detail and one with a pinned clock (screenshots) never
redraw for ages. Pinned by `detail::tests::ages` (render counts over 30 ticks of an old object, one
redraw on a day rollover, a young object, condition and event ticking only on their own tab) and
`detail::ages::tests`.

### The detail's YAML and Describe off the UI thread (E07-P598)

The `detail-drawer` scenario (a 5 MB ConfigMap, its tabs cycled with their keys every half second)
had two frames over budget in every run of the #587 baseline: about 330 ms at the first switch to
YAML and about 40 ms at the first switch to Describe, 44 to 45 refreshes dropped. The YAML tab
serialised the object on the UI thread (`refresh_yaml`) and pushed the text into gpui-component's
editor from `render`, whose `DisplayMap` wrapped every line of the document, twice (once on
`set_value`, again when the font arrived in the first prepaint); the Describe text took the same
path.

Now neither tab does per-line work on the UI thread:

- `yaml_text` runs on the background executor over the `Arc` of the object the view already holds;
  the describe text becomes an `Arc<str>` on the Tokio thread.
- The text is shown in `oxikube_ui::code_view::CodeView`: its row map (soft wrap at the measured
  width, or lines cut at 1 000 columns) and its tree-sitter parse are built on the background
  executor; a `uniform_list` slices, colours and shapes only the rows in the viewport (one run each);
  the colours of the rows around the viewport come from the parse through a two-entry cache. The
  view is mounted under the skeleton from the tab's first frame, so the rows are built once, for the
  final width; a new width re-wraps off the UI thread.
- Memory: one copy of each text (shared by the tab, copy/save and the view), a row map of 24 bytes a
  row and the parser's rope and tree, instead of the editor's rope, display map and per-line wrap
  boundaries.

Switching tabs (`DetailView::set_tab`) costs 0.01 to 0.04 ms. Supplementary, headless (the same
5.5 MB YAML and 5.3 MB describe text, the real macOS text system and Metal renderer, `release-fast`,
busy machine): every draw of the cycle, including the frames that swap in the rows and the colours,
0.1 to 2.0 ms. This is not the acceptance measure; the real-window runs are:

| `detail-drawer`, 5 real-window runs each | frames | max ms | p99 ms | p95 ms | over 8.33 ms | dropped | input max ms | peak RSS MiB |
|---|---|---|---|---|---|---|---|---|
| before (`ea1fce2d`, the #587 baseline in #598) | 131 to 135 | 327.4 to 339.2 | 39.1 to 40.6 | 3.81 to 4.06 | 2 | 44 to 45 | 338.0 to 349.7 | 579.9 to 587.3 |
| after | pending | pending | pending | pending | pending | pending | pending | pending |

The "after" row is taken by the story's verify stage on a quiet machine: during the story's own
session the machine's screen stayed locked, and macOS does not refresh a window on a locked
screen, so every windowed run there reported itself as not a measurement (no frame was taken from
a headless or shortened run instead).

### Memory: 10 000 pods under 400 MB (E07-F508)

Measured on the story branch of E07-S09 the windowed app read **504 MiB** RSS with the 10 000 pods
listed, over the 400 MB budget for 10 k pods
([#508](https://github.com/karan-vk/Oxikube/issues/508)). Every watched object was held twice as a
`serde_json::Value` tree: once in the kube adapter's reflector store (`FeedObject`), and once in the
resource store's cache, which received a clone of it in the `DeltaBatch`. A tree costs about eight
times its JSON: a load pod (`managedFields` stripped) is 2.0 KB of JSON and 16.1 KB in 162
allocations as a `Value` (serde_json `preserve_order`), plus 1.7 KB for its typed `ObjectMeta`.

**Fix:** `Resource::json` is an `Arc<Value>` (ADR 0005, amendment), so the reflector's clone and the
store's copy are one tree; a `Resource` clone now costs its `ObjectMeta` (about 1 KB, two
allocations) instead of about 17 KB. No port changed. Tests that hold it: the domain's
`a_clone_shares_the_json_document`, the feed's `feed::tests::sharing` (the opening list, live changes
and both relist deliveries hand the consumer the reflector store's own document) and the store's
`the_cache_keeps_the_feeds_json_document_without_copying_it` (the tree itself is gone since E07-P603:
see [Memory: compact object storage](#memory-compact-object-storage-e07-p603)).

**Allocator retention** (the issue's third hypothesis) is not the cause: `vmmap` of the running app
with the table listed shows 224 MiB allocated in the default malloc zone with 7 % fragmentation,
i.e. live objects, not pages the allocator kept after the list.

Numbers: same machine and cluster as above, `release-fast`, `--perf --perf-duration 45..60
--perf-table kind-oxikube`, `cargo xtask load-pods --count 10000 --namespaces 8 --churn` running;
the two binaries alternated back to back (main at `ef05d29` against this story). The peak is the
`peak_rss_mib` of the exit summary and is reached, within a few MiB, as the first list lands;
afterwards the reading stays at or near it (and drops further whenever macOS compresses idle pages, which other builds on
the machine caused during some runs, so steady-state readings are only indicative).

| Pods listed | Before: peak RSS | After: peak RSS | After: RSS p50 |
|---|---|---|---|
| 10 990 (run 1) | 542.8 MiB | 342.9 MiB (10 010 pods) | 194.7 MiB (compressed after 30 s) |
| 9 963 / 10 154 (run 2) | 507.3 MiB | 356.9 MiB | 350.2 MiB |
| 19 950 / 19 935 (another story's 10 000 pods on the shared cluster as well) | 868.1 MiB | 552.6 MiB | 306.0 MiB |

So 10 000 pods peak at about 345 to 357 MiB, under the 400 MB budget, and each further 10 000
pods costs about 210 MiB instead of about 330 MiB. Frame times are unchanged (p95 5.9 to 6.9 ms before
and after in these runs, about 100 fps paced by the scroll timer).

The headless `scroll-10k` scenario does not change (`peak_rss_mib` 182.8 before, 181.2 after, 5
samples each): it feeds testkit pods straight into one store, so it never held the second copy.
The remaining cost is the one tree per pod; keeping only column fields hot and the JSON compact or
lazy (the issue's second option) would cut it further but is not needed for the budget.

### Memory: compact object storage (E07-P603)

The idle scenario (two clusters of 1 000 pods, both pods tables open) peaked at 186 MiB against 143
([#603](https://github.com/karan-vk/Oxikube/issues/603)); the two clusters cost about 35.5 MB of
heap, about 17 KB per pod object in 277 000 more allocations, the signature of `serde_json::Value`
trees (one node per field, a `String` per key and per text, a table per object). Since E07-F508 the
tree is shared between the feed's cache and the store's, but it is still a tree.

**What changed** (ADR 0005, second amendment). The issue's fourth idea, handing the allocator's free
pages back after a relist (`malloc_zone_pressure_relief`, `malloc_trim`), was tried and dropped: on macOS
the allocator has already marked the pages of freed blocks reusable by then (a process that freed 90 %
of 400 000 small blocks had a 33 MB footprint against 57 MB resident, and the relief released 0 bytes more),
and resident size, the figure the budget reads, does not move until the kernel needs the pages. The lever
is how many blocks a pod takes, which is what this change cuts:

- `Resource` holds its JSON as a `JsonDoc` (`oxikube_domain::json`): one immutable byte buffer,
  shared between clones, in a tagged varint format with the common Kubernetes keys as one byte and
  every container carrying its byte length. Reads (`Resource::json()`, `get`, `get_str`, the column
  functions, the view-models, the health tally) walk the bytes through `JsonRef` and borrow strings
  from them: no allocation, no decoding. A full `Value` tree is built only on cold paths (the
  editor's `edit_json`, CRD schemas, an event row about the open object, one `status` summary).
- `ObjectMeta` shrank with it: `labels` and `annotations` are `StrMap`s (a sorted shared slice; the
  label set every pod of a ReplicaSet repeats is one allocation, an empty map none) instead of
  `BTreeMap`s (a 380-byte node per map), and the texts many objects repeat (namespace, label keys
  and values, owner references, the `Gvk`) are `intern`ed: one `Arc<str>` per distinct text while an
  object uses it (a weak table, cleaned when it doubles).
- The store's name, namespace and label indices hold a single key inline (`KeySet`) instead of a
  one-element `HashSet` (100 to 150 bytes each, one per pod for the unique names).
- Lists were already streamed into the store (`InitialListStrategy::StreamingList`, paged lists as the
  fallback), one object at a time; each object is now encoded into its compact document as it arrives, so
  nothing list-sized is held as a tree while a list lands.
- No metadata-only mode below the 25 000-object threshold, no fewer pods, nothing dropped but
  `managedFields` (already stripped at ingest by default).

**Measured, in the test suite** (no window needed; exact byte counts, not sampled RSS):

| What | Before | After |
|---|---|---|
| `oxikube_testkit` `heap_per_pod`: one pod as a `Value` tree | 8 450 B in 122 blocks | the document: 533 B in 1 block |
| the same, the whole `Resource` (typed metadata and `Gvk` included) | about 9 400 B (tree + about 1 000 B) | 869 B in 3 blocks |
| `bins/oxikube` `heap_probe`: live heap per pod over the idle scenario's real services, stores and tables, both clusters | 10 400 B | 2 100 to 2 300 B (about 1 000 B of it is the synthetic server's own copy of the pods, which a user's app does not hold) |

That is a cut of about 78 % in what one pod costs across the app, and 90 % in the object itself.
Reproduce with `cargo test -p oxikube_testkit --test heap_per_pod -- --nocapture` and
`cargo test -p oxikube --lib heap_probe -- --nocapture`
(`OXIKUBE_HEAP_PODS=1000 ... heap_probe -- --ignored --nocapture` prints the live bytes the idle
scenario adds at that size, to subtract two sizes by hand). `heap_probe` asserts a ceiling of 5 000 B
per pod, so growth of the per-pod cost fails the suite. The before figure was measured with the same
test on `origin/main` (`ea1fce2d`).

**What it costs in time** (`cargo test -p oxikube_domain --profile release-fast --test json_cost --
--ignored --nocapture`, M-series): encoding a pod when a feed converts it, 5.0 us for `Resource::from_json`
including metadata and a clone of the input tree (10 000 pods: about 50 ms across the feed's task, off the UI
thread); `PodSummary::from_resource` 160 ns; a pointer read of one field 30 to 80 ns; `Resource::clone` 49 ns.
The cell cache means the table reads each visible cell once a second at most.

**Windowed runs** (`cargo xtask perf --windowed idle`, the numbers the budget is judged on) could not
be taken on the machine this story was built on: its screen was locked for the whole session, and
macOS stops refreshing a window nobody can see (the harness reports "not a measurement" and exits).
The verify stage re-measures `idle`, `pods-table` and the 10 000-pod real-kind list on a quiet,
unlocked machine. The per-pod figures above are what those runs should move by: 2 000 pods at about
8 KB less is about 16 MB by this harness's count and up to about 27 MB by the issue's 17 KB per pod
(the harness does not see everything the release app holds), and the 20.8 MB the allocator held dirty but free shrinks with the blocks it
was fragmented by (there are about 40 times fewer of them per pod); the 43 MiB the idle scenario was over is shared with
#604's drawables.

## Log viewer: streaming 5 000 lines/s (E08-S02)

Budget: streaming 5 000 lines/s with p95 frame ≤ 8 ms and no frame > 50 ms; scroll and keypress
≤ 1 frame; memory bounded by `logs.buffer_lines`.

How the view stays inside it (`oxikube_logs_ui::view`): the `LogService` commits lines in batches
(2 048 lines or one 32 ms tick); the view polls one delta per wake (computed at poll time, so a slow
frame gets one larger delta, never a queue) and redraws through `notify_coalesced`; the lines stay in
the session's ring buffer and a frame reads only the rows on screen, by seq, under one short lock;
unwrapped rows are a `uniform_list` and draw at most 1 024 bytes of a line, wrapped rows a `list`
over a `ListState` spliced per delta (only rows on screen are measured). GPUI's line layout cache
reuses the shaping of rows drawn in the previous frame.

### Measuring it

Headless, every run of `cargo xtask perf` (the nightly included): the `logs-stream` scenario
(`bins/oxikube/src/perf_scenario/logs_stream.rs`) runs the real `LogService` and `LogView` on
testkit fakes. The pod's log is a 1 000-line tail, then 5 000 lines a second (request lines of
varying length, every 40th line about 600 bytes) replayed on the log port's clock, one 8.33 ms
frame of it per scripted frame. Six modes run 120 frames each on one stream: wrap off and
following (the budget's mode, `frame_ms`), autoscroll paused (`paused_frame_ms`), wrapped and
following (`wrap_frame_ms`), wrapped and paused (`wrap_paused_frame_ms`), then with a search on (E08-S03, `WARN|ERROR`, about
one line in six, wrap off, following): highlighting (`search_frame_ms`) and filtering
(`filter_frame_ms`). `cargo xtask perf`
fails when any mode's p95 frame is above 8 ms (`xtask/src/perf/budget.rs`; on Linux too since
E07-F509, see [the Linux runner's 340 ms frames](#the-linux-runners-340-ms-frames-509)).

```
cargo xtask perf logs-stream
```

Windowed, against kind: a pod that writes about 5 000 lines/s, then the app opening its log view
through the same commands as a user (`cluster::Connect`, `pod::ViewLogs`, optionally
`logs::ToggleWrap` and `logs::ToggleAutoscroll`):

```
kubectl --context kind-oxikube -n <ns> run firehose --image=registry.k8s.io/e2e-test-images/busybox:1.36.1-1 \
  -- sh -c 'while true; do seq 1 1000 | sed "s/^/INFO fast line /"; sleep 0.2; done'
cargo build -p oxikube --profile release-fast
target/release-fast/oxikube --perf-logs kind-oxikube/<ns>/firehose [--perf-logs-wrap] [--perf-logs-paused] --perf-duration 30
```

`--perf-logs` prints the lines received per second every 5 s; `--perf` prints the frame times and
notify counts on exit.

### Numbers (E08-S02, M-series laptop, release-fast)

Headless (`cargo xtask perf logs-stream`, median of 5 fresh processes, 120 frames per mode;
CPU, layout and paint preparation, no present and no GPU time):

| Mode | lines/s received | frames | p50 | p95 | p99 | max |
|---|---|---|---|---|---|---|
| wrap off, autoscroll on | 5 000 | 144 | 0.56 ms | 0.92 ms | 1.06 ms | 1.09 ms |
| wrap off, autoscroll paused | 5 000 | 144 | 0.57 ms | 0.75 ms | 0.84 ms | 1.11 ms |
| wrap on, autoscroll on | 5 000 | 144 | 0.58 ms | 1.06 ms | 1.11 ms | 1.14 ms |
| wrap on, autoscroll paused | 5 000 | 144 | 0.60 ms | 0.74 ms | 0.84 ms | 0.94 ms |

144 frames per mode: one per scripted frame and one per delta's coalesced notify (24, one per
32 ms commit; at most one notify between two frames). Headless RSS 61.5 MiB with the stream's
lines in the ring buffer.

The `--check` baseline comes from the nightly's own runners, never from the laptop (E08-F520,
[#520](https://github.com/karan-vk/Oxikube/issues/520)): `docs/perf/baseline.json` holds the
`logs-stream` numbers of **nightly run
[37715509006](https://github.com/karan-vk/Oxikube/actions/runs/37715509006)** (`workflow_dispatch`
on `story/E08-F520-perf-baselines`, 7 samples, `release-fast`), seeded with `cargo xtask perf
--from-report perf-report-<OS>/report-<OS>.json --update-baseline` for the `perf-report-Linux` and
`perf-report-macOS` artifacts. Before it, the Linux entry still held the pre-E07-F509 numbers (29.95 ms
p50, 33.2 ms p95 for `frame_ms`), so a Linux regression up to 20x would have passed; a unit test
now fails when a committed `logs-stream` baseline is missing a budgeted mode or is over its budget.

| Mode (p50 / p95 `*_frame_ms`) | `linux` (ubuntu-latest) | `macos` (macos-latest) |
|---|---|---|
| wrap off, autoscroll on (`frame_ms`) | 2.34 / 3.74 ms | 2.19 / 3.69 ms |
| wrap off, autoscroll paused | 2.38 / 2.45 ms | 2.27 / 3.01 ms |
| wrap on, autoscroll on | 1.69 / 3.29 ms | 1.56 / 3.11 ms |
| wrap on, autoscroll paused | 1.73 / 1.79 ms | 1.49 / 1.88 ms |
| search highlighting | 2.61 / 4.03 ms | 2.38 / 3.95 ms |
| search filtering | 2.71 / 4.74 ms | 2.48 / 4.23 ms |
| 10 pods merged, wrap off | 2.65 / 4.11 ms | 2.48 / 3.73 ms |
| `rss_mib` | 196.2 MiB | 112.8 MiB |

Every mode is inside the 8 ms p95 budget on both runners; the CI runners' headless frames are 3 to 5
times the laptop's (the laptop figures above are not comparable with them).

Windowed (`--perf-logs`, 30 s each, the window not in front):

| Mode | lines/s received | frames | p50 | p95 | max | notify |
|---|---|---|---|---|---|---|
| wrap off, autoscroll on | ~4 930 | 18 | 2.42 ms | 9.96 ms | 9.96 ms | 5.2/s, ≤ 3 per frame |
| wrap off, autoscroll paused | ~4 920 | 40 | 2.55 ms | 5.56 ms | 9.15 ms | 5.1/s |
| wrap on, autoscroll on | ~4 880 | 105 | 2.31 ms | 2.71 ms | 9.10 ms | 6.8/s |
| wrap on, autoscroll paused | ~4 920 | 12 | 2.85 ms | 10.10 ms | 10.10 ms | 5.1/s |

RSS stayed at 150-156 MiB with the default 50 000-line buffer full. macOS draws an occluded
window rarely, so these runs drew only 12 to 105 frames. Below 50 frames the p95 is the slowest
single frame, and these frames include the window's first draw of the view, which shapes a whole
screen of rows at once. Every run's max is 9-10 ms, whatever its frame count. So these windowed
runs neither show nor refute the 8 ms p95. The headless scenario measures the streaming frames:
120 per mode, every one under 1.2 ms of CPU. No frame in either measurement came near the 50 ms
limit.

## Log search and filter (E08-S03)

Budget: keystroke to updated highlights <= 1 frame on a full ring buffer (100 000 lines); streaming
5 000 lines/s with a filter on keeps p95 frame <= 8 ms.

How (`oxikube_app::logs::filter`, `oxikube_logs_ui::search`): a `MatchIndex` (the sorted seqs of the
matching lines) rides in the view's `LineWindow`. A delta tests only the lines it appended and drops
the matches of lines the ring dropped, so a streaming frame pays one regex test per new line however
large the ring is. A pattern edit builds a new index: up to 4 000 retained lines on the spot,
otherwise in 16 384-line jobs on the background executor (each holds the session's lock for its own
chunk only), and the finished index is published in one update; until then the old index keeps
serving, so typing never blocks a frame. Highlight spans are computed for the rows on screen at draw
time (a regex over at most 64 spans of the drawn text). Filter mode narrows the window's rows to the
index, so the list stays virtualised over the matches.

Headless, in the `logs-stream` scenario above (`search_*`, `filter_*`; one `cargo xtask perf
logs-stream` run, median of 5 fresh processes, macOS M-series, release-fast, other agents' builds
running on the machine, so noisy; the same run's search-free mode was p95 1.63 ms):

| Mode | lines/s received | frames | p50 | p95 | p99 | max |
|---|---|---|---|---|---|---|
| search highlighting `WARN\|ERROR`, following | 5 000 | 144 | 1.14 ms | 2.08 ms | 2.47 ms | 2.61 ms |
| search filtering `WARN\|ERROR`, following | 5 000 | 144 | 0.94 ms | 2.02 ms | 2.42 ms | 2.68 ms |

The index itself, `cargo bench -p oxikube_app --bench log_search` (CPU only, release): a pattern
edit over a full 100 000-line ring of 100-byte lines tests every line in 0.7 to 3.0 ms in total (7
chunks, the slowest 0.13 to 0.53 ms: the longest the UI thread could wait on the session's lock), and
compiling the pattern takes 17 to 350 µs. One second of streaming (5 000 lines into the full ring,
the oldest dropped) updates the index in 45 to 170 µs. So a keystroke's rescan is far inside a
frame even before it is moved off the UI thread, and the streaming cost is about 0.15 ms per
second of 5 000 lines.

Against a live stream (kind): `cargo test -p oxikube_app --features integration --test kind_smoke
logs_search` follows a pod writing 20 lines/s into a 100-line ring and checks the index equals a
naive scan of what the ring holds.

## Log JSON structured mode (E08-S05)

A line that is a JSON object is drawn as level chip, time, message and collapsed fields; the level
of every line is read once as the service commits it (before the buffer's lock), the columns are
parsed only for the rows on screen and cached by seq, and nothing is parsed per frame.

Parse throughput, one core, `cargo bench -p oxikube_app --bench log_parse` (release, M-series, a
corpus of zap, logrus, bunyan and pino lines about 112 bytes each plus 20 % plain text):

| Cost | lines/s on one core | headroom over 5 000 lines/s |
|---|---|---|
| `classify`: the level, per committed line | ~1 350 000 | x270 |
| `parse_line` + summary: the columns, per row shown | ~900 000 | x180 |

Headless frames (`cargo xtask perf logs-stream`, release-fast, median of 5 fresh processes, 120
frames per mode, 5 000 lines/s of a stream that is three quarters JSON; the lines are longer than
before E08-S05's fixture change, so compare the new modes with each other, not with the E08-S02
table above):

| Mode | p50 | p95 | p99 | max |
|---|---|---|---|---|
| JSON mode on, following (`frame_ms`) | 1.80 ms | 3.58 ms | 4.29 ms | 4.91 ms |
| JSON on, autoscroll paused | 1.82 ms | 3.09 ms | 4.41 ms | 4.52 ms |
| JSON on, wrapped, following | 1.25 ms | 3.04 ms | 4.06 ms | 5.39 ms |
| JSON on, wrapped, paused | 1.25 ms | 1.83 ms | 2.77 ms | 2.82 ms |
| JSON mode off (`raw_frame_ms`) | 1.36 ms | 2.41 ms | 2.59 ms | 3.34 ms |
| JSON on, debug and plain chips off (`json_filtered_frame_ms`) | 2.18 ms | 3.46 ms | 4.32 ms | 4.40 ms |

All within the 8 ms p95 budget (`xtask/src/perf/budget.rs` holds the two new modes); at most one
notify per frame; headless RSS 76 MiB.

The search modes of E08-S03 run over the same JSON-heavy stream with JSON mode on (the level chips
compose with the search: while narrowed, the rows are the matches that also pass the chips, and a
delta tests only its new matches against the chips): search highlighting p95 2.12 ms, search
filtering p95 2.26 ms (same run, headless frames as above, 8 modes, peak RSS 82 MiB).

## Multi-pod aggregation (E08-S04)

Budget: the lines of all the pods of a Deployment, merged, at 5 000 lines/s in total: p95 frame
≤ 8 ms, memory bounded by `logs.buffer_lines` (one merged ring, not one per pod).

How it stays inside it (`oxikube_app::logs::aggregate`): every container is read by its own task
with the single session's batching (2 048 lines or one 32 ms tick) and hands whole batches to one
coordinator over a bounded queue (a few batches per stream: a coordinator that falls behind pushes
back on the connection instead of queueing lines in memory). The coordinator merges them in a
min-heap (`merge.rs`: server timestamp, stream id, position; a line waits one 300 ms reorder
window; the heap is capped at `logs.buffer_lines`, also while the start-up barrier holds the
window) and commits what is due to the one ring buffer in one short critical section per flush.
So the memory is the ring plus at most as many lines again in the merge, whatever the number of
pods. The view is the pod's view plus a gutter per row (a hash lookup per
visible row) and, with sources switched off, a seq index in the window.

Headless (`cargo xtask perf logs-stream`, the `merged_*` modes: the same 5 000 lines/s dealt to the
10 pods of a Deployment, merged through the aggregate session, 120 frames per mode, median of 5
fresh processes, release-fast, M-series laptop under the load of other builds; budget 8 ms p95 on
macOS in `xtask/src/perf/budget.rs`):

| Mode | lines/s received | p50 | p95 | p99 | max |
|---|---|---|---|---|---|
| one pod, wrap off, following (`frame_ms`, for comparison) | 5 000 | 0.73 ms | 1.36 ms | 1.81 ms | 1.94 ms |
| 10 pods merged, wrap off, following (`merged_frame_ms`) | 5 000 | 1.05 ms | 1.87 ms | 2.12 ms | 2.18 ms |
| 10 pods merged, wrapped, following (`merged_wrap_frame_ms`) | 5 000 | 0.98 ms | 1.82 ms | 2.37 ms | 3.09 ms |

Every mode keeps at most one coalesced notify between two frames. The merge adds the reorder
window (300 ms) between a line being written and being shown.

Throughput of the merge itself (`cargo bench -p oxikube_app --bench log_merge`, release, always-ready
synthetic streams, virtual clock; 100-byte lines): 10 pods x 30 000 lines through a 50 000-line ring
in 90 ms (3.3 M lines/s, 667x the budget), 20 pods x 15 000 lines in 138 ms (2.2 M lines/s), 5 pods x
10 000 lines into a 1 000 000-line ring in 14 ms, 10 pods x 30 000 lines through a 5 000-line ring in
58 ms (5.2 M lines/s). None of the kept lines is out of timestamp order. Memory (a counting
allocator: the peak heap the run held; a single session through a 50 000-line ring holds 18 MB in
`log_ingest`): 40.6 MB for 10 pods and a 50 000-line ring, 40.8 MB for 20 pods (flat in the number of
pods), 8.4 MB for a 5 000-line ring; resident memory for the 300 000-line run goes from 3 to 53 MB.

Against kind, windowed (`oxikube --perf-logs kind-oxikube/<ns>/firehose --perf-logs-workload
--perf-duration 40`, release-fast, a Deployment of 10 busybox pods each printing 50 lines every
100 ms): the view received about 4 900 lines/s merged (`oxikube --perf-logs` prints it every 5 s),
RSS 168 MiB p50 / 173 MiB peak with the 50 000-line ring full. As with the single-pod runs, the
window was not in front, so macOS drew 22 frames in the 40 s (p50 2.3 ms, p95 7.0 ms and max 8.6
ms, the first draw included): this run shows the stream keeping up and the memory bounded, and
does not by itself prove the 8 ms p95. The headless scenario above is the measurement of the frames.

```
kubectl --context kind-oxikube create namespace <ns>
kubectl --context kind-oxikube -n <ns> create deployment firehose --replicas=10 \
  --image=registry.k8s.io/e2e-test-images/busybox:1.36.1-1 -- sh -c \
  'while true; do seq 1 50 | sed "s/^/INFO fast line /"; sleep 0.1; done'
target/release-fast/oxikube --perf-logs kind-oxikube/<ns>/firehose --perf-logs-workload --perf-duration 40
```

## Reconnect and churn following (E08-S07)

Budget: a rollout restart (or a reconnect storm) must not spike CPU above the streaming baseline
(idle cost of reconnect bookkeeping ≤ 1 %); dedupe cost bounded per line.

How (`oxikube_app::logs::churn`): the reconnect pauses back off (500 ms doubling to 30 s, plus a
hashed jitter of up to a quarter so streams that broke together do not reopen together) and stop
after `logs.reconnect_retries` failures in a row; a pod that is gone, or a followed container that
finished for good, ends its stream instead of retrying, and a container between restarts is waited
for with the same growing pauses. Dedupe is one hash of the line's text and one set insert per line, over the last 512 lines
of each stream (about 30 KB per stream), whatever the buffer holds.

Measured (M-series laptop, release-fast, shared kind cluster): a 20-replica Deployment writing 10
lines/s per pod, opened with `oxikube --perf --perf-logs kind-oxikube/<ns>/fleet
--perf-logs-workload --perf-duration 75`, `kubectl rollout restart deployment/fleet` 15 s in. The view
followed all 20 new pods (Sources 40: 20 ended, 20 new) at ~197 lines/s throughout; `--perf`: 2 854
frames, p50 2.40 ms, p95 2.78 ms, p99 3.03 ms, max 8.69 ms, 0 dropped, at most 3 notifies per frame,
RSS 157-159 MiB. Process CPU (`ps %cpu`, 1 s samples) was 13-18 % while streaming before the
restart and 5-18 % during it (it dips while the old pods stop): no reconnect spike. The headless
`cargo xtask perf logs-stream` scenario stays inside its budgets with the dedupe on the hot path
(p95 `frame_ms` 3.3 ms, `merged_frame_ms` 4.2 ms on a machine busy with other builds).

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
2. Coalesce notifications: `notify_coalesced` batches `cx.notify()` to the window's frames (one
   per entity per drawn frame, E05-P599); feeds batch watch events (E04-S02).
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
