# ADR 0013: Performance budget — the app must feel as smooth as Zed

- **Status:** Accepted (2026-10-03)
- **Deciders:** project owner
- **Related:** docs/PERFORMANCE.md (budgets + how to measure), ADR 0002 (layers), ADR 0004 (oxikube_ui)

## Context

The reason to build Oxikube natively instead of on Electron is responsiveness. Zed sets the
bar: input-to-pixel latency under one frame, 120 fps on ProMotion displays, instant window
open, and no stalls while background work runs. Lens/OpenLens users' top complaints are high
CPU, freezes and memory (docs/research/features-freelens-k9s.md §D). A GPUI app can still
feel slow if it blocks the main thread, re-renders whole tables, or notifies on every watch
event. Smoothness is therefore a product requirement with numbers, not a polish phase.

## Decision

Oxikube adopts the budgets in `docs/PERFORMANCE.md` as release-gating requirements:

- **Frame time:** p95 ≤ 8 ms (120 Hz) and p99 ≤ 16 ms during scrolling, typing, resizing and
  live watch churn (10 000 pods with 1 %/5 s churn), measured with `--perf`.
- **Input latency:** keystroke/click to visible change within one frame; the command palette
  opens in ≤ 1 frame and filters 2 000 entries in ≤ 5 ms.
- **Startup:** cold launch to first interactive frame ≤ 400 ms on an M-series Mac; catalog
  visible before any cluster connects; cluster tab usable before all feeds are warm.
- **Main thread:** zero network, disk, process or lock-heavy work on the UI thread (enforced by
  the layer rules: `oxikube_app` and adapters run on Tokio via `spawn_kube`; UI receives
  batched deltas); `cx.notify()` coalesced to frame cadence.
- **Memory:** steady-state idle < 150 MB with two clusters connected; 10 k pods < 400 MB;
  bounded ring buffers for logs/events; idle feeds torn down (watch budget).
- **Rendering:** every list/table/log/thread view is virtualised (`uniform_list`/Table);
  no per-frame allocation storms; images/SVGs cached; animations respect reduce-motion.

Every UI story's acceptance criteria inherit these budgets. Stories that touch hot paths
(tables, logs, editor, terminal, agent thread, resource store, feeds) must report measured
numbers in the PR. CI nightly runs the perf scenarios and fails on regression beyond 20 % by default (the hosted runners' nightly gate uses +50 % on p50 and +150 % on p95/p99 because their run-to-run noise is larger than 20 %, and the macOS check is advisory because its noise is larger still; E01-F542). A regression must also exceed an absolute floor per class of metric: 0.25 ms for frames and startup stages, 40 ms for cold-start milestones, 8 MiB for memory (E08-F520).

## Consequences

More instrumentation and benchmark fixtures up front (E01-S14, E05-S13, E07-S09, E10-S11).
Some features ship later because the fast version is harder (e.g. virtualised diff view).
Reviewers reject PRs that block the UI thread or skip virtualisation even when functionally
correct.
