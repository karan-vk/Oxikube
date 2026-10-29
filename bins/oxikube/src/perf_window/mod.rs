//! `oxikube --perf-scenario-window <name>` (feature `perf-window`, E01-P587, ADR 0016): the app,
//! in its real window on the real GPU, drives its own UI through the paths a user's input takes
//! (commands on the bus, actions, keystrokes and scroll events dispatched to the window) while
//! `--perf` records, and writes the scenario's summary next to the JSONL. `cargo xtask perf
//! --windowed` runs them.
//!
//! This is not synthetic OS input (no events are posted to the window server), but everything from
//! the window's event dispatch down is the app's: hit testing, key bindings, focus, the input
//! handler, the views, layout, paint and present. The load comes from synthetic clusters
//! ([`world`]): exact, in real time, no cluster needed. The `terminal` scenario runs a real
//! shell (`LocalPty`), or a shell in a real pod when given one (`--perf-exec`).
//!
//! | Scenario | What it does |
//! |---|---|
//! | `pods-table` | scroll a 10 000-pod table under 1 %/5 s churn, then leave it still |
//! | `table-filter` | type a filter in a 10 000-pod table |
//! | `namespaces` | switch namespace |
//! | `detail-drawer` | open the drawer on a 5 MB ConfigMap and cycle Overview, YAML, Describe, Events |
//! | `tabs-panes` | switch cluster tabs; resize the window and the dock continuously |
//! | `theme` | switch theme |
//! | `catalog` | 50 contexts, type a search |
//! | `sidebar` | count badges under churn |
//! | `nodes-table` | the Nodes table with its pods-by-node index on, under 10 000-pod churn |
//! | `overview` | the cluster overview of a 100-node, 3 000-pod cluster under churn (E13-S09) |
//! | `overview-graphs` | the same overview with live metrics and Prometheus graphs (E13-S09-F1) |
//! | `audit-log` | the audit viewer over a real SQLite log of 100 000 records: fling through it (paging as it goes), then cycle its filters (E19-S09) |
//! | `logs` | stream 5 000 lines/s; type a search; JSON mode |
//! | `terminal` | a 50 MB `yes` flood; a full-screen redraw at 60 Hz; resize |
//! | `idle` | two clusters connected, nothing moving: CPU |
//!
//! | File | Holds |
//! |---|---|
//! | [`world`] | the synthetic clusters |
//! | `driver` | [`Driver`]: what scripts use (commands, waits, keystrokes, scroll events, phases) |
//! | `run` | start-up of a windowed run: the environment, the frame hook's tap, the summary |
//! | `scenarios/*` | one script per scenario |

mod driver;
#[cfg(test)]
mod heap_probe;
pub mod run;
mod scenarios;
#[cfg(test)]
mod tests;
pub mod world;

pub use driver::Driver;
pub use run::{WindowRun, start, startup_env};

use std::time::Duration;

use oxikube_runtime::perf::windowed::Budgets;

use world::{ClusterSpec, WorldSpec};

/// A windowed scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    /// Scroll a 10 000-pod table under churn.
    PodsTable,
    /// Type a filter in a 10 000-pod table.
    TableFilter,
    /// Switch namespace.
    Namespaces,
    /// Cycle the detail drawer's tabs on a 5 MB object.
    DetailDrawer,
    /// Switch cluster tabs, resize the window and the dock.
    TabsPanes,
    /// Switch theme.
    Theme,
    /// 50 contexts, search typing.
    Catalog,
    /// Sidebar count badges under churn.
    Sidebar,
    /// The Nodes table, whose Pods column keeps the pods-by-node index on, under pod churn.
    NodesTable,
    /// The cluster overview of a 100-node, 3 000-pod cluster under churn.
    Overview,
    /// The same overview, with live metrics and Prometheus graphs (E13-S09-F1).
    OverviewGraphs,
    /// The audit viewer over 100 000 records: scroll, then change filters.
    AuditLog,
    /// Stream 5 000 log lines a second; search typing; JSON mode.
    Logs,
    /// `yes` flood, full-screen redraws, resize.
    Terminal,
    /// Two clusters, nothing moving.
    Idle,
}

/// The context of the main synthetic cluster.
pub const MAIN_CONTEXT: &str = "perf-a";
/// Pods of the main synthetic cluster (the budget's 10 000).
pub const PODS: usize = 10_000;
/// Records in the `audit-log` scenario's SQLite audit log (E19-S09).
pub const AUDIT_RECORDS: usize = 100_000;
/// Nodes of the `overview` scenario's cluster (E13-S09's fixture).
pub const OVERVIEW_NODES: usize = 100;
/// Pods of the `overview` scenario's cluster.
pub const OVERVIEW_PODS: usize = 3_000;

impl Scenario {
    /// Every scenario, in report order.
    pub const ALL: [Scenario; 15] = [
        Scenario::PodsTable,
        Scenario::TableFilter,
        Scenario::Namespaces,
        Scenario::DetailDrawer,
        Scenario::TabsPanes,
        Scenario::Theme,
        Scenario::Catalog,
        Scenario::Sidebar,
        Scenario::NodesTable,
        Scenario::Overview,
        Scenario::OverviewGraphs,
        Scenario::AuditLog,
        Scenario::Logs,
        Scenario::Terminal,
        Scenario::Idle,
    ];

    /// The name on the command line and in the report.
    pub fn name(self) -> &'static str {
        match self {
            Scenario::PodsTable => "pods-table",
            Scenario::TableFilter => "table-filter",
            Scenario::Namespaces => "namespaces",
            Scenario::DetailDrawer => "detail-drawer",
            Scenario::TabsPanes => "tabs-panes",
            Scenario::Theme => "theme",
            Scenario::Catalog => "catalog",
            Scenario::Sidebar => "sidebar",
            Scenario::NodesTable => "nodes-table",
            Scenario::Overview => "overview",
            Scenario::OverviewGraphs => "overview-graphs",
            Scenario::AuditLog => "audit-log",
            Scenario::Logs => "logs",
            Scenario::Terminal => "terminal",
            Scenario::Idle => "idle",
        }
    }

    /// The scenario called `name`.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    /// The synthetic clusters it runs against.
    pub fn world(self) -> WorldSpec {
        let main = || ClusterSpec::churning(MAIN_CONTEXT, PODS);
        match self {
            Scenario::PodsTable
            | Scenario::TableFilter
            | Scenario::Namespaces
            | Scenario::DetailDrawer
            | Scenario::Theme
            | Scenario::Sidebar
            | Scenario::NodesTable
            | Scenario::Logs => WorldSpec {
                clusters: vec![main()],
                listed_only: 0,
                audit_records: 0,
            },
            Scenario::Overview => WorldSpec {
                clusters: vec![
                    ClusterSpec::churning(MAIN_CONTEXT, OVERVIEW_PODS).with_nodes(OVERVIEW_NODES),
                ],
                listed_only: 0,
                audit_records: 0,
            },
            // The same cluster, with a metrics-server and a Prometheus-compatible endpoint
            // (E13-S09-F1): the usage bars, the top consumers and the history graphs have data.
            Scenario::OverviewGraphs => WorldSpec {
                clusters: vec![
                    ClusterSpec::churning(MAIN_CONTEXT, OVERVIEW_PODS)
                        .with_nodes(OVERVIEW_NODES)
                        .with_metrics()
                        .with_prometheus(),
                ],
                listed_only: 0,
                audit_records: 0,
            },
            // A quiet cluster and a real SQLite audit log of 100 000 records (E19-S09).
            Scenario::AuditLog => WorldSpec {
                clusters: vec![ClusterSpec::still(MAIN_CONTEXT, 100)],
                listed_only: 0,
                audit_records: AUDIT_RECORDS,
            },
            Scenario::TabsPanes => WorldSpec {
                clusters: vec![
                    main(),
                    ClusterSpec::churning("perf-b", 3_000),
                    ClusterSpec::churning("perf-c", 3_000),
                ],
                listed_only: 0,
                audit_records: 0,
            },
            Scenario::Catalog => WorldSpec {
                clusters: vec![ClusterSpec::still(MAIN_CONTEXT, 100)],
                listed_only: 49,
                audit_records: 0,
            },
            Scenario::Terminal => WorldSpec::default(),
            Scenario::Idle => WorldSpec {
                clusters: vec![
                    ClusterSpec::still(MAIN_CONTEXT, 1_000),
                    ClusterSpec::still("perf-b", 1_000),
                ],
                listed_only: 0,
                audit_records: 0,
            },
        }
    }

    /// The budgets it is judged against: ADR 0016's for every scenario, plus the 10 000-pod memory
    /// budget where the load is the 10 000-pod cluster and its views alone, and for `idle` (two
    /// clusters connected) the idle CPU and idle memory budgets. (The drawer's 5 MB object, the
    /// log stream's buffer and the two extra clusters of `tabs-panes` are more than 10 000 pods;
    /// their peak RSS is reported, not judged.)
    pub fn budgets(self) -> Budgets {
        match self {
            Scenario::PodsTable
            | Scenario::TableFilter
            | Scenario::Namespaces
            | Scenario::Theme
            | Scenario::Sidebar
            | Scenario::NodesTable => Budgets {
                peak_rss_mib: Some(MEMORY_10K_PODS_MIB),
                ..Budgets::default()
            },
            Scenario::Idle => Budgets {
                peak_rss_mib: Some(MEMORY_IDLE_MIB),
                idle_cpu_percent: Some(IDLE_CPU_PERCENT),
                ..Budgets::default()
            },
            _ => Budgets::default(),
        }
    }
}

/// ADR 0016: 10 000 pods under 400 MB (MB = 10^6 bytes, so about 381 MiB).
pub const MEMORY_10K_PODS_MIB: f64 = 400.0 * 1_000_000.0 / 1_048_576.0;
/// ADR 0013 (restated by ADR 0016): idle under 150 MB with two clusters connected (about
/// 143 MiB), judged on the `idle` scenario's peak resident memory, which is never below its
/// steady state.
pub const MEMORY_IDLE_MIB: f64 = 150.0 * 1_000_000.0 / 1_048_576.0;
/// ADR 0016: idle CPU under 1 % with two clusters connected.
pub const IDLE_CPU_PERCENT: f64 = 1.0;
/// How long a scripted phase runs unless it says otherwise.
pub const PHASE: Duration = Duration::from_secs(15);
