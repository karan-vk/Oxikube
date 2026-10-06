//! The tables `cargo xtask perf` prints: the report and the budget check.

use super::budget;
use super::report::{self, Percentiles, Report, Status};

/// Prints the budget table; returns whether a budget failed (and failing is on).
pub fn check_budgets(report: &Report, skip: bool) -> bool {
    let rows = budget::check(report, budget::BUDGETS);
    if rows.is_empty() {
        return false;
    }
    println!(
        "
budgets (p95 across cold launches{}):",
        if skip { ", not enforced" } else { "" }
    );
    for row in &rows {
        println!("  {row}");
    }
    !skip && rows.iter().any(|row| row.verdict == budget::Verdict::Fail)
}

pub fn print_report(report: &Report) {
    println!(
        "\n{} {} ({}), median of {} samples per scenario. {}",
        report.os, report.arch, report.profile, report.samples_per_scenario, report.note
    );
    println!(
        "{:<12} {:<26} {:>10} {:>10} {:>10} {:>10}",
        "scenario", "metric (unit)", "p50", "p95", "p99", "max"
    );
    for (name, result) in &report.scenarios {
        match result.status {
            Status::NotAvailable => println!(
                "{name:<12} SKIPPED: not available yet ({}); enabled by {}",
                result.reason.as_deref().unwrap_or("-"),
                result.enabled_by.join(", ")
            ),
            Status::Ok => {
                for (metric, p) in &result.metrics {
                    println!("{}", row(name, metric, p));
                }
                for (metric, p) in &result.launches {
                    println!(
                        "{}  across {} launches",
                        row(name, metric, p),
                        result.samples
                    );
                }
                let c = result.counters;
                println!(
                    "{name:<12} counters: {} frames, {} dropped, {} feed deltas, {} notifies",
                    c.frames, c.dropped_frames, c.feed_deltas, c.notifies
                );
            }
        }
    }
}

/// One metric row of the report table.
fn row(scenario: &str, metric: &str, p: &Percentiles) -> String {
    format!(
        "{scenario:<12} {:<26} {:>10.3} {:>10.3} {:>10.3} {:>10}",
        format!("{metric} [{}]", report::metric_unit(metric)),
        p.p50,
        p.p95,
        p.p99,
        p.max.map(|m| format!("{m:.3}")).unwrap_or_default()
    )
}
