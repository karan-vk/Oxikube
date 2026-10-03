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
- `cargo xtask load-pods --count 10000 --churn` seeds the churn scenario on kind (E01-S10); see
  [Load fixture](#load-fixture-cargo-xtask-load-pods) below.
- `cargo xtask perf <scenario>` runs scripted scenarios headless and writes a report; nightly
  CI compares against `docs/perf/baseline.json` and fails on > 20 % regression (E01-S14).
- macOS: Instruments (Time Profiler, Metal System Trace) for stalls; Linux: `perf` + `tracy`
  via the `tracy` feature on `oxikube_runtime`.
- Memory: `cargo xtask perf memory` samples RSS; leaks checked with the GPUI
  `leak-detection` feature in tests.

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
| `--cleanup` | | delete the load namespaces (`<prefix>` and `<prefix>-<digits>` only) and exit |
| `--context C` | `kind-oxikube` | kubectl context; anything not starting with `kind-` is refused |
| `--allow-non-kind` | off | override the guard (you almost certainly do not want this) |

Every kubectl call carries `--context`, so the tool never follows your current context. Pods are
applied in chunks of 500 with progress output (about 120 pods/s on a laptop, so 10 000 pods take
roughly 90 s). Ctrl-C stops the churn loop after the current tick, recreates whatever that tick
deleted so the fleet is whole, and exits 0; a second Ctrl-C aborts at once. The tool is idempotent:
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
