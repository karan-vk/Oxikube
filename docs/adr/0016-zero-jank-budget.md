# ADR 0016: Zero-jank budget, measured in the real window

- **Status:** Accepted (2026-10-08)
- **Deciders:** project owner, story E01-P587 (#587)
- **Supersedes:** the frame-time row of ADR 0013 (its input, startup, main-thread, rendering and
  memory rows stand, with the memory and idle CPU rows restated below)
- **Related:** docs/PERFORMANCE.md (budgets, how to measure, the baseline), ADR 0013 (performance
  budget), ADR 0002 (layers)

## Context

ADR 0013 set the frame budget as "p95 ≤ 8 ms, p99 ≤ 16 ms, no frame > 50 ms". That lets janky
frames through: at 120 Hz, a run where one frame in twenty takes 15 ms meets it while visibly
stuttering, and a 40 ms frame (five missed refreshes) passes. A percentile budget also hides the
frames a user notices most, the rare long ones on an input. The owner's bar is stricter: every view
renders with zero dropped or janky frames, measured honestly in the real window, and stays
efficient on CPU and memory.

The measurement had the same gap. Only the pods table (`--perf-table`) and the log viewer
(`--perf-logs`) had windowed runs, both against a live kind cluster whose load varies; everything
else was measured headless (GPUI's test platform: no present, no GPU), which says nothing about the
frames a user sees.

## Decision

### The budget

During every scenario below, on the reference Mac (Apple M-series laptop, its built-in 120 Hz
display, `release-fast`, nothing else running):

| What | Budget | Judged on |
|---|---|---|
| Frames | every frame drawn in ≤ **8.33 ms** (one 120 Hz refresh) | the **maximum** (and p99, p95 reported); not a percentile |
| Dropped frames | **0** refreshes missed while a view is being driven | the gaps between the display refreshes the window is called on |
| Input | ≤ **1 frame**: dispatched at a refresh, shown in the frame of that refresh | from the dispatch to the end of the paint of the frame that shows it, ≤ 8.33 ms |
| Memory | 10 000 pods **< 400 MB**; idle **< 150 MB** with two clusters connected (ADR 0013's rows) | the process's peak resident memory (for idle, never below its steady state) |
| Idle CPU | **< 1 %** with two clusters connected | process CPU time over an idle phase |
| Notifies | ≤ **1 coalesced notify per view per frame** | the most one view received between two frames |

A frame is judged from the start of `Window::draw` to the end of the content's paint: layout,
prepaint and paint of the whole tree and of its deferred overlays (popovers, menus, dropdowns), the
app's own work for that frame. The hook marks that end with a probe drawn as the last deferred
draw. GPUI paints the window's one tooltip, in-window prompt or drag preview after every deferred
draw, so that element's paint (not its layout or prepaint) falls outside the judged time; it is in
`presented_ms`, and a long one shows as a dropped refresh. The `--perf` hook also
reports the frame to the end of `present` (`presented_ms`), but that is not judged: GPUI's Metal
`present` waits for a free drawable, so while the window draws on every refresh it lasts about until
the next refresh (a windowed pods-table scroll reads about 8.3 ms per presented frame for about
3 to 4 ms of drawing). It measures the display's pacing, not the app. Whatever does run long there
(a heavy scene to encode, a GPU that cannot keep up) or anywhere else on the main thread makes the
window miss a refresh, and the dropped-frame budget, zero, catches it. The input budget is judged the
same way: the frame that shows an input must be painted within the refresh it was dispatched at.

### The measurement

`oxikube --perf-scenario-window <name>` (feature `perf-window`), run by `cargo xtask perf
--windowed`, starts the real app in its real window and has it drive its own UI through the paths
a user's input takes: commands on the command bus, actions, keystrokes and scroll events
dispatched to the window (`Window::dispatch_keystroke`, `Window::dispatch_event`), window and dock
resizes. This is not synthetic OS input (nothing is posted to the window server, so hit testing,
key bindings, focus, the input handler and everything below run as for a user, but the OS event
queue is not involved), and it is the real window on the real GPU.

- **Pacing.** A scripted phase runs one step on every display refresh (GPUI's `on_next_frame`,
  called from the platform's display link before it draws), so the UI changes as often as the
  display can show it, as under a trackpad fling or a held key. A refresh the window misses shows
  as a gap of more than one and a half refresh intervals between two calls: those are the dropped
  frames. The refresh interval is measured before the scenario (the median gap while nothing is
  drawn) and reported.
- **Phases.** Setup (connecting, opening views, the first list) is reported apart and not judged:
  start-up and cluster open have their own budgets (ADR 0013). Every frame of every scripted phase
  is judged; none is skipped.
- **Load.** The scenarios run against synthetic clusters (`bins/oxikube/src/perf_window/world`): the
  app's own session manager, stores, services and views over ports that produce exactly the
  scenario's load in real time (10 000 pods with `load-pods --churn`'s 1 % every 5 s, 5 000 log
  lines a second, a 5 MB object). Only what a cluster would send is synthetic; nothing a user's
  frames depend on is skipped or cached. The terminal scenario runs a real shell (`LocalPty`), or
  a shell in a real kind pod over the kube adapter's exec when kind is available.
- **Validity.** GPUI paces an inactive window at 30 fps, so a run during which the window was not
  the active window for even one refresh is marked `valid: false` and is not a measurement.
- **Output.** Each run writes the `--perf` JSONL as before plus a per-scenario summary (max, p99,
  p95, dropped, input latency, notifies per frame and per view, RSS, CPU, every over-budget frame
  with its phase and time) and a verdict against this budget.

The scenarios: the pods table (scroll 10 000 pods under 1 %/5 s churn), the table filter (typing),
namespaces (switching), the detail drawer (Overview, YAML on a 5 MB object, Describe, Events),
tabs and panes (switch cluster tabs; resize the window and the dock continuously), the theme
(switching), the catalog (50 contexts, search typing), the sidebar (count badges under churn), the
log viewer (5 000 lines/s with search typing and with JSON mode), the terminal (a 50 MB `yes` flood;
a full-screen redraw at 60 Hz; resize) and idle (two clusters).

### Rules

1. The budget is not loosened to pass, and a scenario is not shrunk, its load not lowered, its
   frames not excluded, and it is not measured only headless. A frame over budget is fixed at its
   cause (work off the UI thread, incremental updates, virtualisation, layout caching, fewer
   allocations, coalesced notifies, cheaper shaping and paint).
2. Every over-budget frame of the baseline is attributed to a cause (tracing spans and a time
   profile) in docs/PERFORMANCE.md, and fixes are stories of their own.
3. A story that touches a hot path reports the windowed numbers of the scenarios it affects,
   before and after (several runs, the same machine).

## Consequences

- Most views do not meet this budget today (the baseline in docs/PERFORMANCE.md); the gap is the
  backlog of fix stories filed from it. The headless scenarios and their nightly gate (ADR 0013,
  E08-F520) stay: they catch regressions on CI runners, which have no display.
- Windowed runs need a quiet machine with the window in front; they are a local and pre-release
  measurement, not a CI gate.
- `--perf` gained a per-view notify figure (`max_view_notifies_per_frame`, JSONL schema 2) and a
  frame tap the windowed driver attributes frames with.
