//! `helm::SearchCharts`, `helm::ShowChart` and `helm::InstallChart` on the command bus (tool
//! stubs `helm.search_charts`, `helm.show_chart`, `helm.install_chart`). `helm::OpenCharts` opens
//! the page and is registered by `oxikube_helm_ui::charts`.
//!
//! All three are read-only: search and show only read repositories through `helm`, and
//! `helm::InstallChart` only hands a chart to the install tab, whose install goes through
//! `MutationGuard`.

use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::helm::{ChartRef, ChartSummary};
use oxikube_domain::{OxiError, OxiResult};
use serde_json::{Value, json};

use super::service::ChartsService;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};

/// The commands [`register_commands`] installs.
pub const COMMANDS: [CommandId; 3] = [
    CommandId::HELM_SEARCH_CHARTS,
    CommandId::HELM_SHOW_CHART,
    CommandId::HELM_INSTALL_CHART,
];

/// Registers the three commands on `registry`; the binary installs it as `oxikube_app::helm_charts`.
///
/// # Errors
///
/// A [`RegisterError`] when an id is already taken.
pub fn register_commands(
    registry: &mut CommandRegistry,
    service: ChartsService,
) -> Result<(), RegisterError> {
    for id in COMMANDS {
        let meta = *command::lookup(id).expect("declared in oxikube_domain::command");
        let service = service.clone();
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let service = service.clone();
            async move { run(&service, command).await }
        })?;
    }
    Ok(())
}

fn summary_json(chart: &ChartSummary) -> Value {
    let versions: Vec<_> = chart
        .versions
        .iter()
        .map(|v| json!({ "version": v.version, "app_version": v.app_version, "description": v.description }))
        .collect();
    json!({
        "repo": chart.repo,
        "name": chart.name,
        "latest": chart.latest().version,
        "description": chart.latest().description,
        "versions": versions,
    })
}

fn chart_ref(repo: String, name: String, version: Option<String>) -> ChartRef {
    ChartRef::new(name, version.unwrap_or_default()).in_repo(repo)
}

pub(super) async fn run(service: &ChartsService, command: Command) -> OxiResult<CommandOutput> {
    match command {
        Command::HelmSearchCharts { keyword } => {
            let charts = service.search(&keyword).await?;
            Ok(CommandOutput {
                message: Some(format!("{} charts", charts.len())),
                data: Some(
                    json!({ "charts": charts.iter().map(summary_json).collect::<Vec<_>>() }),
                ),
            })
        }
        Command::HelmShowChart {
            repo,
            name,
            version,
            part,
        } => {
            let chart = chart_ref(repo, name, version);
            let text = service.detail(&chart, part).await?;
            Ok(CommandOutput {
                message: None,
                data: Some(json!({ "chart": chart.reference(), "text": &*text })),
            })
        }
        Command::HelmInstallChart {
            repo,
            name,
            version,
        } => {
            let chart = chart_ref(repo, name, version);
            let message = format!("Install {}", chart.reference());
            service.request_install(chart);
            Ok(CommandOutput {
                message: Some(message),
                data: None,
            })
        }
        other => Err(OxiError::internal(format!(
            "{} is not a Helm charts command",
            other.id()
        ))),
    }
}
