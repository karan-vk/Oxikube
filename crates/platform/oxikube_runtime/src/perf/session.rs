//! `--perf` session: a background thread drains the [`Recorder`] and appends JSONL.
//!
//! File: `<dir>/oxikube-perf-<unix ms>-<pid>.jsonl`, where `<dir>` is chosen by the caller (the
//! binary defaults to `<data dir>/perf`, honouring `OXIKUBE_DATA_DIR`). One JSON object per line:
//!
//! - `{"kind":"start", "schema", "app_version", "os", "arch", "pid", "started_unix_ms", "flush_interval_ms", "measures"}`
//! - `{"kind":"tick", "t_ms", "interval_ms", "frames_us":[..], "dropped_frames", "feed_deltas", "feed_deltas_per_s", "notifies", "notifies_per_s", "max_notifies_per_frame", "rss_mib", "peak_rss_mib"}`
//!   every flush interval (one per second by default), `frames_us` holding every frame drawn in it;
//!   `rss_mib` / `peak_rss_mib` are the process's resident memory (MiB) read on the flush thread
//!   when the tick is written (`null` where the OS has no reader, see [`memory`]);
//! - `{"kind":"summary", ...SessionSummary}` once, from [`PerfSession::finish`].
//!
//! The UI thread never touches the file or reads memory: it only records into the lock-free
//! [`Recorder`].

use super::memory::{self, MemoryReading, bytes_to_mib};
use super::recorder::{Recorder, Tick};
use super::stats::{Summary, round_ms};
use serde::Serialize;
use serde_json::json;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Version of the JSONL line format.
pub const JSONL_SCHEMA: u32 = 1;

/// How often the flush thread drains the recorder and writes a `tick` line.
pub const DEFAULT_FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// What a recorded frame covers (written into the `start` line and the reports).
pub const FRAME_MEASURES: &str = "root view render start -> end of the GPUI update that drew (and, \
in the windowed app, presented) the frame; excludes GPU execution and display latency";

/// Totals for a whole session; printed on exit and written as the final JSONL line.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionSummary {
    /// Wall time of the session, ms.
    pub duration_ms: f64,
    /// Frames drawn.
    pub frame_count: u64,
    /// Frame-time distribution; `None` when no frame was drawn.
    pub frames: Option<Summary>,
    /// Frames lost to ring overflow (should be 0).
    pub dropped_frames: u64,
    /// Feed deltas applied.
    pub feed_deltas: u64,
    /// Feed throughput over the session.
    pub feed_deltas_per_s: f64,
    /// Coalesced notifies.
    pub notifies: u64,
    /// Notify rate over the session.
    pub notifies_per_s: f64,
    /// The most coalesced notifies delivered between two consecutive frames (1 with one streaming
    /// view on screen; see [`Recorder`]).
    pub max_notifies_per_frame: u64,
    /// Resident memory over the per-tick readings, MiB; `None` when the OS has no reader.
    pub rss_mib: Option<Summary>,
    /// Peak resident memory of the process (OS high-water mark), MiB.
    pub peak_rss_mib: Option<f64>,
}

impl fmt::Display for SessionSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let secs = self.duration_ms / 1000.0;
        match &self.frames {
            Some(s) => write!(
                f,
                "{} frames in {secs:.1} s: p50 {:.3} ms, p95 {:.3} ms, p99 {:.3} ms, max {:.3} ms",
                self.frame_count, s.p50, s.p95, s.p99, s.max
            )?,
            None => write!(f, "0 frames in {secs:.1} s (no window was redrawn)")?,
        }
        write!(
            f,
            "; dropped {}; feed {} deltas ({:.1}/s); notify {} ({:.1}/s, at most {} per frame)",
            self.dropped_frames,
            self.feed_deltas,
            self.feed_deltas_per_s,
            self.notifies,
            self.notifies_per_s,
            self.max_notifies_per_frame
        )?;
        if let Some(rss) = &self.rss_mib {
            write!(
                f,
                "; rss p50 {:.1} MiB, p95 {:.1} MiB, max {:.1} MiB",
                rss.p50, rss.p95, rss.max
            )?;
        }
        if let Some(peak) = self.peak_rss_mib {
            write!(f, ", peak {peak:.1} MiB")?;
        }
        Ok(())
    }
}

/// Running totals across ticks.
#[derive(Default)]
struct Accumulator {
    frames_ns: Vec<u64>,
    dropped: u64,
    feed: u64,
    notify: u64,
    max_notifies_per_frame: u64,
    rss_bytes: Vec<u64>,
    peak_rss_bytes: Option<u64>,
}

impl Accumulator {
    fn add(&mut self, tick: &Tick) {
        self.frames_ns.extend_from_slice(&tick.frames_ns);
        self.dropped += tick.dropped_frames;
        self.feed += tick.feed_deltas;
        self.notify += tick.notifies;
        self.max_notifies_per_frame = self.max_notifies_per_frame.max(tick.max_notifies_per_frame);
    }

    fn add_memory(&mut self, reading: Option<MemoryReading>) {
        let Some(reading) = reading else { return };
        self.rss_bytes.push(reading.rss_bytes);
        let peak = reading.peak_rss_bytes.unwrap_or(reading.rss_bytes);
        self.peak_rss_bytes = Some(self.peak_rss_bytes.map_or(peak, |p| p.max(peak)));
    }

    fn summary(&mut self, elapsed: Duration) -> SessionSummary {
        let secs = elapsed.as_secs_f64().max(f64::EPSILON);
        SessionSummary {
            duration_ms: round_ms(elapsed.as_secs_f64() * 1000.0),
            frame_count: self.frames_ns.len() as u64,
            frames: Summary::from_nanos(&mut self.frames_ns),
            dropped_frames: self.dropped,
            feed_deltas: self.feed,
            feed_deltas_per_s: per_second(self.feed, secs),
            notifies: self.notify,
            notifies_per_s: per_second(self.notify, secs),
            max_notifies_per_frame: self.max_notifies_per_frame,
            rss_mib: Summary::from_scaled(&mut self.rss_bytes, memory::MIB),
            peak_rss_mib: self.peak_rss_bytes.map(bytes_to_mib),
        }
    }
}

fn per_second(n: u64, secs: f64) -> f64 {
    (n as f64 / secs * 10.0).round() / 10.0
}

fn start_line(app_version: &str, interval: Duration) -> serde_json::Value {
    let started_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    json!({
        "kind": "start",
        "schema": JSONL_SCHEMA,
        "app_version": app_version,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "pid": std::process::id(),
        "started_unix_ms": started_unix_ms,
        "flush_interval_ms": interval.as_millis() as u64,
        "measures": FRAME_MEASURES,
    })
}

fn tick_line(
    tick: &Tick,
    since_start: Duration,
    memory: Option<MemoryReading>,
) -> serde_json::Value {
    let secs = tick.interval.as_secs_f64().max(f64::EPSILON);
    let frames_us: Vec<u64> = tick.frames_ns.iter().map(|ns| ns / 1000).collect();
    json!({
        "kind": "tick",
        "t_ms": since_start.as_millis() as u64,
        "interval_ms": tick.interval.as_millis() as u64,
        "frames_us": frames_us,
        "dropped_frames": tick.dropped_frames,
        "feed_deltas": tick.feed_deltas,
        "feed_deltas_per_s": per_second(tick.feed_deltas, secs),
        "notifies": tick.notifies,
        "notifies_per_s": per_second(tick.notifies, secs),
        "max_notifies_per_frame": tick.max_notifies_per_frame,
        "rss_mib": memory.map(|m| bytes_to_mib(m.rss_bytes)),
        "peak_rss_mib": memory.and_then(|m| m.peak_rss_bytes).map(bytes_to_mib),
    })
}

fn summary_line(summary: &SessionSummary) -> serde_json::Value {
    let mut line = serde_json::Map::new();
    line.insert("kind".into(), "summary".into());
    if let Ok(serde_json::Value::Object(fields)) = serde_json::to_value(summary) {
        line.extend(fields);
    }
    serde_json::Value::Object(line)
}

fn write_line(out: &mut impl Write, value: &serde_json::Value) -> io::Result<()> {
    serde_json::to_writer(&mut *out, value)?;
    out.write_all(b"\n")?;
    out.flush()
}

/// Result of [`PerfSession::finish`].
#[derive(Debug)]
pub struct Finished {
    /// Session totals (always available, even if writing failed).
    pub summary: SessionSummary,
    /// The JSONL file.
    pub path: PathBuf,
    /// First write error, if the file could not be fully written.
    pub error: Option<io::Error>,
}

/// A running `--perf` session (owns the flush thread).
pub struct PerfSession {
    path: PathBuf,
    stop: mpsc::Sender<()>,
    thread: JoinHandle<(SessionSummary, Option<io::Error>)>,
}

impl PerfSession {
    /// Creates `dir` and the JSONL file, writes the `start` line, and spawns the flush thread
    /// (`oxikube-perf`), which drains `recorder` every `interval`.
    ///
    /// Call before the GPUI app runs (the file is created on the calling thread).
    pub fn start(
        recorder: Arc<Recorder>,
        dir: &Path,
        interval: Duration,
        app_version: &str,
    ) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let started_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let path = dir.join(format!(
            "oxikube-perf-{started_ms}-{}.jsonl",
            std::process::id()
        ));
        let mut out = BufWriter::new(File::create(&path)?);
        write_line(&mut out, &start_line(app_version, interval))?;

        let (stop, stopped) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("oxikube-perf".into())
            .spawn(move || flush_loop(&recorder, out, interval, &stopped, memory::read))?;
        Ok(Self { path, stop, thread })
    }

    /// The JSONL file being written.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Stops the flush thread after a final drain, writes the `summary` line and returns the
    /// totals.
    pub fn finish(self) -> Finished {
        let _ = self.stop.send(());
        let (summary, error) = match self.thread.join() {
            Ok(result) => result,
            Err(_) => (
                Accumulator::default().summary(Duration::ZERO),
                Some(io::Error::other("perf flush thread panicked")),
            ),
        };
        Finished {
            summary,
            path: self.path,
            error,
        }
    }
}

fn flush_loop(
    recorder: &Recorder,
    mut out: impl Write,
    interval: Duration,
    stopped: &mpsc::Receiver<()>,
    sample_memory: impl Fn() -> Option<MemoryReading>,
) -> (SessionSummary, Option<io::Error>) {
    let started = Instant::now();
    let mut reader = recorder.reader();
    let mut acc = Accumulator::default();
    let mut error = None;
    loop {
        let stop = !matches!(
            stopped.recv_timeout(interval),
            Err(RecvTimeoutError::Timeout)
        );
        let tick = reader.drain(recorder);
        acc.add(&tick);
        // Read on this thread, once per tick: a syscall or a /proc read has no place in a frame.
        let memory = sample_memory();
        acc.add_memory(memory);
        if error.is_none() {
            error = write_line(&mut out, &tick_line(&tick, started.elapsed(), memory)).err();
        }
        if stop {
            break;
        }
    }
    let summary = acc.summary(started.elapsed());
    if error.is_none() {
        error = write_line(&mut out, &summary_line(&summary)).err();
    }
    (summary, error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn keys(v: &Value) -> Vec<&str> {
        let mut k: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        k.sort_unstable();
        k
    }

    #[test]
    fn tick_line_schema_and_rates() {
        let tick = Tick {
            frames_ns: vec![1_000_000, 2_500_000],
            dropped_frames: 0,
            feed_deltas: 500,
            notifies: 60,
            max_notifies_per_frame: 1,
            interval: Duration::from_millis(500),
        };
        let memory = MemoryReading {
            rss_bytes: 120 * 1024 * 1024,
            peak_rss_bytes: Some(130 * 1024 * 1024 + 524_288),
        };
        let line = tick_line(&tick, Duration::from_millis(1500), Some(memory));
        assert_eq!(
            keys(&line),
            [
                "dropped_frames",
                "feed_deltas",
                "feed_deltas_per_s",
                "frames_us",
                "interval_ms",
                "kind",
                "max_notifies_per_frame",
                "notifies",
                "notifies_per_s",
                "peak_rss_mib",
                "rss_mib",
                "t_ms"
            ]
        );
        assert_eq!(line["rss_mib"], json!(120.0));
        assert_eq!(line["peak_rss_mib"], json!(130.5));
        let none = tick_line(&tick, Duration::ZERO, None);
        assert!(none["rss_mib"].is_null() && none["peak_rss_mib"].is_null());
        assert_eq!(line["frames_us"], json!([1000, 2500]));
        assert_eq!(line["feed_deltas_per_s"], json!(1000.0));
        assert_eq!(line["notifies_per_s"], json!(120.0));
    }

    #[test]
    fn session_writes_start_ticks_and_summary() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = Arc::new(Recorder::new());
        let session = PerfSession::start(
            recorder.clone(),
            &dir.path().join("perf"),
            Duration::from_secs(3600),
            "0.0.0-test",
        )
        .unwrap();
        for ms in [2, 4, 6, 8] {
            recorder.record_frame(Duration::from_millis(ms));
        }
        recorder.record_feed_deltas(100);
        recorder.record_notify();
        let path = session.path().to_owned();
        let finished = session.finish();
        assert!(finished.error.is_none(), "{:?}", finished.error);
        assert_eq!(finished.path, path);
        let s = &finished.summary;
        assert_eq!(s.frame_count, 4);
        assert_eq!(s.frames.unwrap().p50, 4.0);
        assert_eq!(s.frames.unwrap().p99, 8.0);
        assert_eq!((s.feed_deltas, s.notifies, s.dropped_frames), (100, 1, 0));

        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let kinds: Vec<&str> = lines.iter().map(|l| l["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["start", "tick", "summary"]);
        assert_eq!(lines[0]["schema"], json!(JSONL_SCHEMA));
        assert_eq!(lines[1]["frames_us"], json!([2000, 4000, 6000, 8000]));
        assert_eq!(lines[2]["frame_count"], json!(4));
        // Real reader: present on Linux and macOS, `null` elsewhere.
        assert_eq!(
            lines[1]["rss_mib"].is_f64(),
            cfg!(any(target_os = "linux", target_os = "macos"))
        );
        assert_eq!(lines[2]["frames"]["p95"], json!(8.0));
        assert!(
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with(".jsonl")
        );
        assert!(
            s.to_string()
                .contains("p50 4.000 ms, p95 8.000 ms, p99 8.000 ms")
        );
    }

    /// Memory is read on the thread running `flush_loop` (the `oxikube-perf` thread in a real
    /// session), once per drain, and lands in the tick and in the summary.
    #[test]
    fn memory_is_sampled_on_the_flush_thread() {
        use std::sync::Mutex;
        const MIB: u64 = 1024 * 1024;
        let sampled_on = Arc::new(Mutex::new(Vec::new()));
        let seen = sampled_on.clone();
        let recorder = Recorder::new();
        let (tx, rx) = mpsc::channel();
        tx.send(()).unwrap(); // already stopped: exactly one drain, one reading, then the summary
        let thread = std::thread::Builder::new()
            .name("oxikube-perf".into())
            .spawn(move || {
                let mut out = Vec::new();
                flush_loop(&recorder, &mut out, Duration::from_secs(3600), &rx, || {
                    seen.lock()
                        .unwrap()
                        .push(std::thread::current().name().map(str::to_owned));
                    Some(MemoryReading {
                        rss_bytes: 100 * MIB,
                        peak_rss_bytes: Some(150 * MIB),
                    })
                });
                out
            })
            .unwrap();
        let out = thread.join().unwrap();
        assert_eq!(
            *sampled_on.lock().unwrap(),
            [Some("oxikube-perf".to_owned())]
        );
        let lines: Vec<Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines[0]["kind"], "tick");
        assert_eq!(lines[0]["rss_mib"], json!(100.0));
        assert_eq!(lines[0]["peak_rss_mib"], json!(150.0));
        assert_eq!(lines[1]["kind"], "summary");
        assert_eq!(lines[1]["rss_mib"]["count"], json!(1));
        assert_eq!(lines[1]["peak_rss_mib"], json!(150.0));
    }

    /// An OS without a reader writes `null`, never a fake zero.
    #[test]
    fn missing_memory_reader_writes_null() {
        let recorder = Recorder::new();
        let (tx, rx) = mpsc::channel();
        tx.send(()).unwrap();
        let mut out = Vec::new();
        let (summary, error) =
            flush_loop(&recorder, &mut out, Duration::from_secs(1), &rx, || None);
        assert!(error.is_none());
        assert_eq!((summary.rss_mib, summary.peak_rss_mib), (None, None));
        let text = String::from_utf8(out).unwrap();
        let tick: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert!(tick["rss_mib"].is_null() && tick["peak_rss_mib"].is_null());
    }

    #[test]
    fn accumulator_memory_summary_and_display() {
        const MIB: u64 = 1024 * 1024;
        let mut acc = Accumulator::default();
        for (rss, peak) in [(100, 120), (110, 120), (130, 140), (120, 140)] {
            acc.add_memory(Some(MemoryReading {
                rss_bytes: rss * MIB,
                peak_rss_bytes: Some(peak * MIB),
            }));
        }
        acc.add_memory(None);
        let s = acc.summary(Duration::from_secs(4));
        let rss = s.rss_mib.unwrap();
        assert_eq!(
            (rss.count, rss.p50, rss.p95, rss.max),
            (4, 110.0, 130.0, 130.0)
        );
        assert_eq!(s.peak_rss_mib, Some(140.0));
        assert!(
            s.to_string()
                .ends_with("; rss p50 110.0 MiB, p95 130.0 MiB, max 130.0 MiB, peak 140.0 MiB")
        );
        let v = summary_line(&s);
        assert_eq!(v["rss_mib"]["p50"], json!(110.0));
        assert_eq!(v["peak_rss_mib"], json!(140.0));
    }

    #[test]
    fn summary_without_frames_says_so() {
        let summary = Accumulator::default().summary(Duration::from_secs(30));
        assert_eq!(summary.frames, None);
        assert!(summary.to_string().starts_with("0 frames in 30.0 s"));
    }
}
