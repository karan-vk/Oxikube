//! `cargo xtask` — repository automation.
//!
//! Subcommands:
//! - `lint-deps`      enforce the hexagonal dependency direction (see docs/ARCHITECTURE.md)
//! - `check-gpui-pin` verify gpui-pre / gpui-component pins are exact and aligned
//! - `kind-up` / `kind-down`  local kind cluster for integration tests (E01-S09)
//! - `load-pods`      create N pause pods (+ optional churn) for perf work (E01-S10)
#![allow(clippy::print_stdout)]

mod check_gpui_pin;
mod kind;
mod lint_deps;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xtask", about = "Oxikube repository automation")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Enforce the hexagonal dependency direction across workspace crates.
    LintDeps,
    /// Verify gpui-pre* and gpui-component pins are exact and mutually aligned.
    CheckGpuiPin,
    /// Create a local kind cluster with metrics-server and test fixtures.
    KindUp {
        #[arg(long, default_value = "oxikube")]
        name: String,
    },
    /// Delete the local kind cluster.
    KindDown {
        #[arg(long, default_value = "oxikube")]
        name: String,
    },
    /// Create N pause pods across namespaces; `--churn` keeps deleting/recreating them.
    LoadPods {
        #[arg(long, default_value_t = 1000)]
        count: usize,
        #[arg(long)]
        churn: bool,
        #[arg(long, default_value = "oxikube-load")]
        namespace: String,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::LintDeps => lint_deps::run(),
        Cmd::CheckGpuiPin => check_gpui_pin::run(),
        Cmd::KindUp { name } => kind::up(&name),
        Cmd::KindDown { name } => kind::down(&name),
        Cmd::LoadPods {
            count,
            churn,
            namespace,
        } => kind::load_pods(count, churn, &namespace),
    }
}
