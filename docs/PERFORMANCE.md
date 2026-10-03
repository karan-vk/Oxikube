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
  `~/.local/share/oxikube/perf/*.jsonl` and prints p50/p95/p99 on exit (E01-S14).
- `cargo xtask load-pods --count 10000 --churn` seeds the churn scenario on kind (E01-S10).
- `cargo xtask perf <scenario>` runs scripted scenarios headless and writes a report; nightly
  CI compares against `docs/perf/baseline.json` and fails on > 20 % regression (E01-S14).
- macOS: Instruments (Time Profiler, Metal System Trace) for stalls; Linux: `perf` + `tracy`
  via the `tracy` feature on `oxikube_runtime`.
- Memory: `cargo xtask perf memory` samples RSS; leaks checked with the GPUI
  `leak-detection` feature in tests.

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
