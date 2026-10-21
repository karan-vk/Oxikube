//! Chart repositories, chart search and `helm show`.

/// A configured chart repository (`helm repo list`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoEntry {
    /// The alias (`bitnami`).
    pub name: String,
    /// The index URL.
    pub url: String,
}

/// One chart version found by a search (`helm search repo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartHit {
    /// The chart, with its repository and this version.
    pub chart: super::release::ChartRef,
    /// The application version this chart version packages.
    pub app_version: Option<String>,
    /// The chart's one-line description.
    pub description: String,
}

/// What to search the configured repositories for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchQuery {
    /// Keyword; empty lists every chart.
    pub keyword: String,
    /// Every version of each chart (`--versions`) instead of only the latest.
    pub all_versions: bool,
}

/// Which part of a chart `helm show` prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShowKind {
    /// `Chart.yaml`.
    Chart,
    /// The README, markdown.
    Readme,
    /// The default `values.yaml`.
    Values,
    /// The custom resource definitions the chart ships.
    Crds,
    /// All of the above.
    All,
}

impl ShowKind {
    /// The `helm show` sub-command.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Chart => "chart",
            Self::Readme => "readme",
            Self::Values => "values",
            Self::Crds => "crds",
            Self::All => "all",
        }
    }
}
