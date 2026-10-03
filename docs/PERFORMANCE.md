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
- Memory: RSS sampling (`cargo xtask perf memory`) is a follow-up to E01-S14 and does not exist
  yet; leaks are checked with the GPUI `leak-detection` feature in tests.

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
| `tick` (every second) | `t_ms`, `interval_ms`, `frames_us` (every frame in the interval), `dropped_frames`, `feed_deltas`, `feed_deltas_per_s`, `notifies`, `notifies_per_s` |
| `summary` (on exit) | `duration_ms`, `frame_count`, `frames` {`count`, `p50`, `p95`, `p99`, `max`} (ms), `dropped_frames`, `feed_deltas`, `feed_deltas_per_s`, `notifies`, `notifies_per_s` |

On exit (window closed, `--perf-duration` elapsed, or Ctrl-C) it prints to stderr, for example:

```
oxikube --perf: 2 frames in 3.4 s: p50 0.593 ms, p95 8.385 ms, p99 8.385 ms, max 8.385 ms; dropped 0; feed 0 deltas (0.0/s); notify 0 (0.0/s)
```

Percentiles are nearest-rank (p99 of fewer than 100 frames is the maximum). Until feeds and
`notify_coalesced` land (E04, E07) nothing calls the feed and notify counters, so they read 0.

Recorder overhead (M-series, release, `cargo run --release -p oxikube_runtime --example
perf_overhead`): a frame push is about 3 ns and the hook's timing pair about 45 ns; a feed or
notify call costs about 0.4 ns with `--perf` off and 2.5 ns with it on. End to end, the startup
scenario's `draw_ms` with and without the hook (`--perf-no-probe`) differs by under 1 µs per frame
(p50 0.009 vs 0.008 ms, median of 15 runs).

### `cargo xtask perf`

```
cargo xtask perf startup               # one scenario, 5 samples
cargo xtask perf --all --check         # every scenario, compare with the baseline
cargo xtask perf --all --update-baseline
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
| `startup` | measured | `first_frame_ms` (first line of `main` to the end of the update that drew the first frame: headless app context, text system, GPU renderer, window, first draw); `launch_to_first_frame_ms` (process spawn to the first-frame marker on stdout, so exec and dynamic loading are included; timed by xtask); `frame_ms` / `draw_ms` (120 idle redraws of the main view: hook time, and wall time of the whole update measured outside GPUI) |
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

`--check` fails when any p50/p95/p99 of a baselined metric is more than **+20 %** slower **and** more
than **0.25 ms** slower (`--tolerance`, `--noise-floor-ms`). The absolute floor stops microsecond
jitter on sub-millisecond metrics (an idle redraw is about 0.01 ms) from failing the job; it is far
below any budget in the table above. A scenario or metric with no baseline is reported as
`MISSING` and does not fail; a scenario that has a baseline but no longer runs does fail.

The nightly `perf` job (ubuntu + macOS) runs `cargo xtask perf --all --check --samples 7`, uploads
`perf-report-<OS>` and, on failure, feeds the `nightly-failure` tracking issue.

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
