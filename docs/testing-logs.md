# The logs test matrix (E08)

Logs are stream processing with timing, ordering and memory bounds, so regressions are silent.
This is the map of what pins which behaviour, where it lives and how to run it. When you change
the log path, run the row that matches, and when you add behaviour, add it here.

All unit and view tests are deterministic: a hand-driven executor and `FakeClockPort` for the app
layer (no runtime, no threads, no sleeps), `#[gpui::test]` with the deterministic runtime for the
views. Fakes: `FakeLogPort` (scripted `Timeline`s: timed lines, errors at a point, graceful end,
`keep_open`), `FakeResourcePort` + `ScriptedFeed` (pod-set changes), `FakeClockPort`,
`FakeFsPort`.

| Behaviour | Test | Where | Run |
|---|---|---|---|
| Buffer bound, truncated marker, seq numbers | `ring::*`, `lifecycle::the_merged_buffer_is_one_ring...` | `oxikube_app` `src/logs/tests/ring.rs`, `aggregate/tests/lifecycle.rs` | `cargo test -p oxikube_app --lib logs::` |
| Delta batching, slow readers, cancel on drop | `batching::*`, `delta::*`, `cancel::*` | `oxikube_app` `src/logs/tests/` | same |
| Reconnect backoff, retry cap, overlap dedupe (a replayed line once; same text at another timestamp kept) | `reconnect::*` (incl. `identical_text_at_other_timestamps...`), `overlap::tests` | `oxikube_app` `src/logs/churn/` | same |
| Pod replaced / finished / rollout following | `fate::*`, `replacement::*`, `quiet::*` | `oxikube_app` `src/logs/churn/tests/` | same |
| Aggregation order: interleaved stamps, ties, out-of-order inside the window, late stream, pod added and removed, recorded three-pod log | `ordering::*`, `merge::tests`, `pods::*`, fixtures `three-pods.log` / `.merged` | `oxikube_app` `src/logs/aggregate/` | same |
| Parser corpus: zap, logrus, bunyan, pino, plus a mixed stream with malformed and plain lines, each against committed normalised records | `tests/logs_corpus` (`expected/*.json`), field-by-field in `src/logs/parse/tests` | `oxikube_app` | `cargo test -p oxikube_app --test logs_corpus` (after an intended parser change: `OXIKUBE_BLESS=1`, review the diff) |
| View: virtualised rows (only visible lines built), truncated marker, state rows | `stream::*`, `local::*` | `oxikube_logs_ui` `src/view/tests/` | `cargo test -p oxikube_logs_ui` |
| View: autoscroll, the new-lines pill, wrap-toggle anchor | `scroll::*` | same | same |
| View: search highlight, next/previous, filter mode, regex errors | `search::*` | same | same |
| View: JSON columns, level chips, expand | `json::*` | same | same |
| View: export dialog and save, copy, selection, marks, clear | `save::*`, `selection`/`copy`/`clear` tests in `src/view/` | same | same |
| View: merged multi-pod, banners, reconnect and follow-the-replacement | `aggregate::*`, `churn::*` | same | same |
| Picture: single pod (levels, wrapped, light), search, filter, selection and marks, JSON with an expanded line, merged pods, and the overview (pod prefixes, JSON columns, expanded line, search highlight) | `tests/screenshot.rs`, goldens in `tests/goldens/<os>/` | `oxikube_logs_ui` | `cargo test -p oxikube_logs_ui --features screenshot --test screenshot` (needs a GPU device; nightly; `OXIKUBE_UPDATE_GOLDENS=1` regenerates) |
| Real adapters on kind: single pod, search, aggregate, churn (rollout restart, no duplicate lines), agent excerpt | `tests/kind_smoke/logs*.rs` (churn: `logs_churn.rs`) | `oxikube_app` | `OXIKUBE_TEST_CONTEXT=kind-oxikube cargo test -p oxikube_app --features integration --test kind_smoke` |

## Mutation check

The dedupe and ordering rules are guarded by more than one test each. To see that, break a rule
and watch the suite fail, then revert:

- `churn/overlap.rs`: `self.counts.contains_key(&key)` to `false` (nothing is deduped), or the
  key's timestamp to a constant (same text is the same line).
- `aggregate/merge.rs`, `impl Ord for Pending`: compare only `key.0` (ties unresolved), compare
  `arrived` (arrival order instead of server order), or compare `(ts, seq, stream)`.
- `parse/level.rs`: move a numeric level threshold; the corpus files fail.
