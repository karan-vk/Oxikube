//! Scripted frame driver for headless perf scenarios (feature `perf-harness`).
//!
//! Works with the GPUI test-mode contexts: `gpui::HeadlessAppContext` (real text system, used by
//! `oxikube --perf-scenario`) and `gpui::TestAppContext` (used by `#[gpui::test]`). In test mode GPUI
//! draws every dirty window when an update finishes flushing its effects (`App::flush_effects`),
//! which stands in for the platform's frame callback. Each scripted frame is one window update:
//! the scenario's `step` applies its input (feed batch, keystrokes, scroll), the window is marked
//! for refresh, and the flush at the end of that update draws exactly one frame. Then the executor
//! is parked. Two timings come out:
//!
//! - `frame_ms`: the [`PerfRoot`](super::PerfRoot) hook, the same code path `oxikube --perf` uses;
//! - `draw_ms`: wall time of the whole update (step, effect flush, draw), measured from outside
//!   (this is what the hook's overhead is judged against).
//!
//! Nothing here sleeps or starts threads, so it is deterministic under GPUI's test scheduler.
//! Resident memory ([`metric::RSS_MIB`]) is read between frames, outside the timed update: there
//! is no flush thread in a headless run, and a reading never lands inside `frame_ms` or `draw_ms`.

use super::memory::{self, MIB};
use super::recorder::{Recorder, RecorderReader};
use super::report::{Counters, ScenarioSample};
use super::stats::Summary;
use anyhow::Result;
use gpui::{AnyWindowHandle, App, AppContext, Window};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// Metric names used in [`ScenarioSample::metrics`].
pub mod metric {
    /// Process start (or scenario start) to the first frame drawn.
    pub const FIRST_FRAME_MS: &str = "first_frame_ms";
    /// Per-frame time from the `PerfRoot` hook.
    pub const FRAME_MS: &str = "frame_ms";
    /// Per-frame wall time of the whole draw update, measured outside GPUI.
    pub const DRAW_MS: &str = "draw_ms";
    /// Spawn of the process to its first-frame marker on stdout (added by `cargo xtask perf`).
    pub const LAUNCH_TO_FIRST_FRAME_MS: &str = "launch_to_first_frame_ms";
    /// Resident memory after each scripted frame, MiB. Headless process RSS: no swap chain,
    /// no GPU surfaces, no windowing-system state, so below the windowed app's.
    pub const RSS_MIB: &str = "rss_mib";
    /// OS high-water mark of resident memory at the end of the run, MiB.
    pub const PEAK_RSS_MIB: &str = "peak_rss_mib";
}

/// Applies `step` to `window`, marks it for refresh and lets the end-of-update flush draw it
/// once (test-mode contexts only). Returns the wall time of the whole update.
pub fn draw_frame<C: AppContext>(
    cx: &mut C,
    window: AnyWindowHandle,
    step: impl FnOnce(&mut Window, &mut App),
) -> Result<Duration> {
    let start = Instant::now();
    cx.update_window(window, |_, window, cx| {
        step(window, cx);
        window.refresh();
    })?;
    Ok(start.elapsed())
}

/// What a scripted run measured.
#[derive(Debug, Default)]
pub struct FrameRun {
    /// `draw_frame` wall times, ns.
    pub draw_ns: Vec<u64>,
    /// Frame-hook times, ns (empty when the window has no `PerfRoot`).
    pub frame_ns: Vec<u64>,
    /// Counters accumulated over the run.
    pub counters: Counters,
    /// Resident memory after each scripted frame, bytes (empty where the OS has no reader).
    pub rss_bytes: Vec<u64>,
    /// OS high-water mark of resident memory at the end of the run, bytes.
    pub peak_rss_bytes: Option<u64>,
}

impl FrameRun {
    /// Builds the sample for `scenario`, adding `extra` metrics (e.g. `first_frame_ms`).
    pub fn into_sample(
        mut self,
        scenario: &str,
        extra: impl IntoIterator<Item = (String, Summary)>,
    ) -> ScenarioSample {
        let mut metrics: BTreeMap<String, Summary> = extra.into_iter().collect();
        if let Some(s) = Summary::from_nanos(&mut self.draw_ns) {
            metrics.insert(metric::DRAW_MS.into(), s);
        }
        if let Some(s) = Summary::from_nanos(&mut self.frame_ns) {
            metrics.insert(metric::FRAME_MS.into(), s);
        }
        if let Some(s) = Summary::from_scaled(&mut self.rss_bytes, MIB) {
            metrics.insert(metric::RSS_MIB.into(), s);
        }
        if let Some(peak) = self.peak_rss_bytes {
            metrics.insert(
                metric::PEAK_RSS_MIB.into(),
                Summary::single(peak as f64 / MIB),
            );
        }
        ScenarioSample::ok(scenario, metrics, self.counters)
    }
}

/// Drives `frames` scripted frames of `window`.
///
/// `reader` must be positioned after any warm-up frames the caller does not want counted (take a
/// fresh `recorder.reader()` and drain it once). `park` runs the executor until idle
/// (`run_until_parked`); `step(frame, window, cx)` applies the scripted input for that frame.
pub fn run_frames<C: AppContext>(
    cx: &mut C,
    window: AnyWindowHandle,
    frames: usize,
    recorder: &Recorder,
    reader: &mut RecorderReader,
    park: impl Fn(&C),
    mut step: impl FnMut(usize, &mut Window, &mut App),
) -> Result<FrameRun> {
    let mut run = FrameRun::default();
    for frame in 0..frames {
        let elapsed = draw_frame(cx, window, |window, cx| step(frame, window, cx))?;
        run.draw_ns
            .push(u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX));
        park(cx);
        if let Some(reading) = memory::read() {
            run.rss_bytes.push(reading.rss_bytes);
            run.peak_rss_bytes = reading.peak_rss_bytes.or(run.peak_rss_bytes);
        }
    }
    let tick = reader.drain(recorder);
    run.counters = Counters {
        frames: tick.frames_ns.len() as u64,
        dropped_frames: tick.dropped_frames,
        feed_deltas: tick.feed_deltas,
        notifies: tick.notifies,
    };
    run.frame_ns = tick.frames_ns;
    Ok(run)
}

#[cfg(test)]
mod tests {
    //! A scripted scenario against a fake feed, through the same driver `oxikube
    //! --perf-scenario` uses, proving the scripted path and the report schema.

    use super::*;
    use crate::perf::{PerfRoot, REPORT_SCHEMA, ScenarioStatus};
    use gpui::{
        Context, Entity, IntoElement, ParentElement, Render, Styled, TestAppContext, Window, div,
    };
    use std::sync::Arc;

    /// Deterministic stand-in for a watch feed: batches of pod-name deltas, a rolling window of
    /// rows like a churned table.
    struct FakeFeed {
        next: usize,
        batch: usize,
    }

    impl FakeFeed {
        fn next_batch(&mut self) -> Vec<String> {
            let rows = (self.next..self.next + self.batch)
                .map(|i| format!("oxikube-load-{}/load-{i}", i % 4))
                .collect();
            self.next += self.batch;
            rows
        }
    }

    struct FeedTable {
        rows: Vec<String>,
    }

    impl FeedTable {
        const VISIBLE: usize = 40;

        fn apply(&mut self, batch: Vec<String>) {
            self.rows.extend(batch);
            let excess = self.rows.len().saturating_sub(Self::VISIBLE);
            self.rows.drain(..excess);
        }
    }

    impl Render for FeedTable {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .flex()
                .flex_col()
                .children(self.rows.iter().map(|row| div().child(row.clone())))
        }
    }

    const FRAMES: usize = 30;
    const BATCH: usize = 25;

    #[gpui::test]
    fn scripted_feed_scenario_produces_a_stable_report(cx: &mut TestAppContext) {
        let recorder = Arc::new(Recorder::new());
        let mut table: Option<Entity<FeedTable>> = None;
        let window = cx.add_window(|_, cx| {
            let inner = cx.new(|_| FeedTable { rows: Vec::new() });
            table = Some(inner.clone());
            PerfRoot::new(inner, recorder.clone())
        });
        let table = table.unwrap();
        let window: AnyWindowHandle = window.into();

        // Warm-up frame, not counted.
        let mut reader = recorder.reader();
        draw_frame(cx, window, |_, _| {}).unwrap();
        cx.run_until_parked();
        reader.drain(&recorder);

        let mut feed = FakeFeed {
            next: 0,
            batch: BATCH,
        };
        let run = run_frames(
            cx,
            window,
            FRAMES,
            &recorder,
            &mut reader,
            |cx| cx.run_until_parked(),
            |_, _, cx| {
                let batch = feed.next_batch();
                let n = batch.len() as u64;
                table.update(cx, |t, cx| {
                    t.apply(batch);
                    cx.notify();
                });
                recorder.record_feed_deltas(n);
                recorder.record_notify();
            },
        )
        .unwrap();

        assert_eq!(run.draw_ns.len(), FRAMES);
        assert_eq!(
            run.counters,
            Counters {
                frames: FRAMES as u64,
                dropped_frames: 0,
                feed_deltas: (FRAMES * BATCH) as u64,
                notifies: FRAMES as u64,
            },
            "one hook frame per scripted draw, every delta and notify counted"
        );
        table.read_with(cx, |t, _| {
            assert_eq!(t.rows.len(), FeedTable::VISIBLE);
            assert_eq!(t.rows.last().unwrap(), "oxikube-load-1/load-749");
        });

        let sample = run.into_sample(
            "fake-feed",
            [(metric::FIRST_FRAME_MS.to_owned(), Summary::single(1.0))],
        );
        assert_eq!(sample.status, ScenarioStatus::Ok);
        assert_eq!(sample.metrics[metric::FRAME_MS].count, FRAMES as u64);
        assert_eq!(sample.metrics[metric::DRAW_MS].count, FRAMES as u64);
        if cfg!(any(target_os = "linux", target_os = "macos")) {
            let rss = sample.metrics[metric::RSS_MIB];
            assert_eq!(rss.count, FRAMES as u64);
            assert!(rss.p50 > 1.0 && rss.p50 <= rss.p99, "{rss:?}");
            assert!(sample.metrics[metric::PEAK_RSS_MIB].p50 >= rss.p50);
        } else {
            assert!(!sample.metrics.contains_key(metric::RSS_MIB));
        }

        // Schema: the same keys, at every level, as the committed example `cargo xtask perf` reads.
        let produced = serde_json::to_value(&sample).unwrap();
        let example: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../docs/perf/scenario-sample.example.json"
        ))
        .unwrap();
        assert_eq!(produced["schema"], REPORT_SCHEMA);
        assert_eq!(key_paths(&produced), key_paths(&example));
    }

    /// Sorted `a.b.c` key paths of a JSON object (metric names collapsed to `*`).
    fn key_paths(v: &serde_json::Value) -> Vec<String> {
        fn walk(prefix: &str, v: &serde_json::Value, out: &mut Vec<String>) {
            if let Some(map) = v.as_object() {
                for (k, child) in map {
                    let k = if prefix == "metrics" { "*" } else { k.as_str() };
                    let path = if prefix.is_empty() {
                        k.to_owned()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    out.push(path.clone());
                    walk(&path, child, out);
                }
            }
        }
        let mut out = Vec::new();
        walk("", v, &mut out);
        out.sort();
        out.dedup();
        out
    }
}
