//! The table `cargo xtask perf --windowed` prints: per scenario, the median and the worst run of
//! each figure ADR 0016 judges, then every budget failure and every run that could not measure.

use super::Report;

/// Prints `report`'s table.
pub fn table(report: &Report) {
    println!(
        "\n{} {} ({}), {} runs per scenario, real window; median run / worst run (ADR 0016: \
         every frame <= 8.33 ms, 0 dropped, input <= 1 frame, <= 1 notify per view per frame)",
        report.os, report.arch, report.profile, report.runs_per_scenario
    );
    println!(
        "{:<14} {:>5} {:>15} {:>15} {:>15} {:>13} {:>15} {:>9} {:>15} {:>13}",
        "scenario",
        "valid",
        "frame max",
        "frame p99",
        "frame p95",
        "dropped",
        "input max",
        "notif/vw",
        "peak RSS MiB",
        "CPU %"
    );
    for (name, s) in &report.scenarios {
        let f = |key: &str| {
            s.figures.get(key).map_or_else(
                || "-".to_owned(),
                |v| format!("{:.2}/{:.2}", v.median, v.worst),
            )
        };
        let n = |key: &str| {
            s.figures.get(key).map_or_else(
                || "-".to_owned(),
                |v| format!("{:.0}/{:.0}", v.median, v.worst),
            )
        };
        let cpu = if s.figures.contains_key("idle_cpu_percent") {
            f("idle_cpu_percent")
        } else {
            f("cpu_percent")
        };
        println!(
            "{name:<14} {:>5} {:>15} {:>15} {:>15} {:>13} {:>15} {:>9} {:>15} {:>13}",
            format!("{}/{}", s.valid_runs, s.runs.len()),
            f("frame_max_ms"),
            f("frame_p99_ms"),
            f("frame_p95_ms"),
            n("dropped_frames"),
            f("input_latency_max_ms"),
            n("max_view_notifies_per_frame"),
            f("peak_rss_mib"),
            cpu
        );
        for failure in &s.failures {
            println!("{:<14} over: {failure}", "");
        }
        for error in &s.errors {
            println!("{:<14} not measured: {error}", "");
        }
    }
}
