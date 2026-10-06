//! `cargo xtask` — repository automation.
//!
//! Subcommands:
//! - `setup`          install git pre-commit and pre-push hooks (E01-S12)
//! - `lint-deps`      enforce the hexagonal dependency direction (see docs/ARCHITECTURE.md)
//! - `check-gpui-pin` verify gpui-pre / gpui-component pins are exact and aligned
//! - `kind-up` / `kind-down`  local kind cluster for integration tests, with the test images pre-pulled (E01-S09, E04-B01)
//! - `load-pods`      create N pause pods (+ optional churn) for perf work (E01-S10)
//! - `perf`           headless perf scenarios, report, baseline check (E01-S14)
//! - `gen-settings-schema`  write (or `--check`) settings.schema.json from the `oxikube` binary (E05-S06, E05-S06b)
#![allow(clippy::print_stdout)]

mod check_gpui_pin;
mod kind;
mod kind_images;
mod lint_deps;
mod load_pods;
mod perf;
mod settings_schema;
mod setup;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xtask", about = "Oxikube repository automation")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Install git pre-commit and pre-push hooks via pre-commit.
    Setup,
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
    LoadPods(load_pods::Args),
    /// Run headless perf scenarios; `--check` gates on docs/perf/baseline.json (+20 %).
    Perf(perf::Args),
    /// Write settings.schema.json from the registered settings; `--check` fails when stale.
    GenSettingsSchema(settings_schema::Args),
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::Setup => setup::run(),
        Cmd::LintDeps => lint_deps::run(),
        Cmd::CheckGpuiPin => check_gpui_pin::run(),
        Cmd::KindUp { name } => kind::up(&name),
        Cmd::KindDown { name } => kind::down(&name),
        Cmd::LoadPods(args) => load_pods::run(&args),
        Cmd::Perf(args) => perf::run(&args),
        Cmd::GenSettingsSchema(args) => settings_schema::run(&args),
    }
}
