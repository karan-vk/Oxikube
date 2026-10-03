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
| Startup | ≤ 400 ms cold to first interactive frame; catalog before any network | `oxikube` launch with 3 kubeconfigs, 20 contexts |
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
  [Load fixture](#load-fixture-cargo-xtask-load-pods) below.
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
| `tick` (every second) | `t_ms`, `interval_ms`, `frames_us` (every frame in the interval), `dropped_frames`, `feed_deltas`, `feed_deltas_per_s`, `notifies`, `notifies_per_s`, `rss_mib`, `peak_rss_mib` (MiB, `null` where the OS has no reader) |
| `summary` (on exit) | `duration_ms`, `frame_count`, `frames` {`count`, `p50`, `p95`, `p99`, `max`} (ms), `dropped_frames`, `feed_deltas`, `feed_deltas_per_s`, `notifies`, `notifies_per_s`, `rss_mib` {`count`, `p50`, `p95`, `p99`, `max`} (MiB, over the per-tick readings), `peak_rss_mib` |

On exit (window closed, `--perf-duration` elapsed, or Ctrl-C) it prints to stderr, for example:

```
oxikube --perf: 2 frames in 4.5 s: p50 0.651 ms, p95 6.343 ms, p99 6.343 ms, max 6.343 ms; dropped 0; feed 0 deltas (0.0/s); notify 0 (0.0/s); rss p50 95.3 MiB, p95 95.3 MiB, max 95.3 MiB, peak 95.3 MiB
```

Percentiles are nearest-rank (p99 of fewer than 100 frames is the maximum). Until feeds and
`notify_coalesced` land (E04, E07) nothing calls the feed and notify counters, so they read 0.

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
  scenario sits at about 31 MiB on an M-series Mac while `oxikube --perf` in a real window reads
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
| `startup` | measured | `rss_mib` / `peak_rss_mib` (headless resident memory after the 120 redraws, MiB; see [Memory (RSS)](#memory-rss)); `first_frame_ms` (first line of `main` to the end of the update that drew the first frame: headless app context, text system, GPU renderer, window, first draw); `launch_to_first_frame_ms` (process spawn to the first-frame marker on stdout, so exec and dynamic loading are included; timed by xtask); `frame_ms` / `draw_ms` (120 idle redraws of the main view: hook time, and wall time of the whole update measured outside GPUI) |
| `scroll-10k` | not available: needs E05-S11 #93, E07-S01 #107, E07-S03 #109 | frame time scrolling the 10 k-pod table under churn |
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
nightly run 37124550799 on the story branch E01-S14b), with a local M-series laptop run for
reference (not gated):

| Metric | `linux` (ubuntu-latest) p50 / p99 | `macos` (macos-latest) p50 / p99 | local M5 Max (9 samples) p50 / p99 |
|---|---|---|---|
| `launch_to_first_frame_ms` | 97.9 / 97.9 | 65.2 / 65.2 | 74.0 / 74.0 |
| `first_frame_ms` | 95.7 / 95.7 | 56.2 / 56.2 | 68.3 / 68.3 |
| `frame_ms` (idle redraw, hook) | 0.249 / 0.284 | 0.010 / 0.052 | 0.006 / 0.011 |
| `draw_ms` (idle redraw, outside) | 0.250 / 0.287 | 0.011 / 0.058 | 0.006 / 0.011 |
| `rss_mib` (headless, after the redraws) | 110.1 / 110.2 | 27.6 / 27.6 | 31.3 / 31.3 |
| `peak_rss_mib` (headless) | 110.2 / 110.2 | 27.6 / 27.6 | 31.3 / 31.3 |

For scale: `oxikube --perf` with a real window on the same laptop reads about 95 MiB RSS, three times
the headless figure, which is why the headless number is only a regression signal. The Linux
runner's headless figure is about four times the macOS runner's (different renderer and system
libraries; not investigated further), another reason baselines are per OS.

The placeholder window is trivial, so these mostly measure platform, text-system and renderer
start-up. The startup budget (≤ 400 ms to the first *interactive* frame with real catalog data) is
E05-S13's job, built on this harness.

Rules for updating the baseline:

1. A PR that adds a scenario (or a metric) seeds it: dispatch the nightly on the branch
   (`gh workflow run nightly.yml --ref <branch>`), download both `perf-report-<OS>` artifacts and
   run `cargo xtask perf --from-report <file> --update-baseline` for each. Commit the result.
2. A PR that makes something intentionally slower (or much faster) refreshes the affected
   scenarios the same way and says so, with before/after numbers, in its Performance section.
3. Never edit numbers by hand and never refresh the baseline to make a red nightly green without
   explaining the regression.


## Load fixture: `cargo xtask load-pods`

The perf fixture for the E07 table/feed stories and the E01-S14 harness. It creates pause pods
(`registry.k8s.io/pause:3.10`, 1m CPU / 1Mi requests, label `app=oxikube-load`) against the local
kind cluster from `cargo xtask kind-up`.

```
cargo xtask kind-up
cargo xtask load-pods --count 10000 --namespaces 8 --churn   # the budget scenario
cargo xtask load-pods --cleanup                              # delete the oxikube-load-* namespaces
```

| Flag | Default | Meaning |
|---|---|---|
| `--count N` | 1000 | pods to create, named `load-0 .. load-(N-1)` |
| `--namespaces N` | 4 | spread round-robin over `<prefix>-0 .. <prefix>-(N-1)` |
| `--namespace PREFIX` | `oxikube-load` | namespace name prefix |
| `--churn` | off | every 5 s delete and recreate about 1 % of the pods (min 1), cycling through all namespaces, until Ctrl-C |
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
