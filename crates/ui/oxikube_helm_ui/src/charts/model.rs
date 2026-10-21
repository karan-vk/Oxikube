//! The page as plain data: what a card says, the detail's tabs and load states.

use oxikube_domain::helm::ChartSummary;

/// Where the page is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Looking for `helm` and the repositories.
    Checking,
    /// No usable `helm`: guidance instead of the catalog.
    NoHelm,
    /// `helm` works but has no repositories.
    NoRepos,
    /// Reading the repositories failed.
    Failed(String),
    /// The catalog is usable; `repos` repositories are configured.
    Ready {
        /// How many repositories are configured.
        repos: usize,
    },
}

/// A detail tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    /// The chart's README.
    Readme,
    /// Every version found.
    Versions,
    /// The chart's default values.
    Values,
}

impl Tab {
    /// Every tab, in display order.
    pub const ALL: [Tab; 3] = [Tab::Readme, Tab::Versions, Tab::Values];

    /// The tab's label.
    pub const fn title(self) -> &'static str {
        match self {
            Tab::Readme => "README",
            Tab::Versions => "Versions",
            Tab::Values => "Default values",
        }
    }

    /// A stable lower-case key (for element ids).
    pub const fn key(self) -> &'static str {
        match self {
            Tab::Readme => "readme",
            Tab::Versions => "versions",
            Tab::Values => "values",
        }
    }
}

/// How far a detail text is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Load {
    /// Not asked for yet (its tab was not opened).
    #[default]
    Idle,
    /// On its way.
    Loading,
    /// Arrived (and shown).
    Ready,
    /// Failed, with a readable reason.
    Failed(String),
}

/// What a catalog card says: strings built once per result, not per frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    /// `repo/name`.
    pub reference: String,
    /// The chart name.
    pub name: String,
    /// The repository alias, or empty.
    pub repo: String,
    /// The one-line description (of the newest version).
    pub description: String,
    /// The newest chart version.
    pub version: String,
    /// The newest version's application version, or empty.
    pub app_version: String,
    /// How many versions were found.
    pub versions: usize,
    /// The icon placeholder's letter.
    pub initial: char,
    /// The icon placeholder's hue in degrees (0..360), from the name (stable across sessions).
    pub hue: u16,
}

impl Card {
    /// The card of `chart`.
    pub fn of(chart: &ChartSummary) -> Self {
        let latest = chart.latest();
        let hash = chart
            .name
            .bytes()
            .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
        Self {
            reference: chart.reference(),
            name: chart.name.clone(),
            repo: chart.repo.clone().unwrap_or_default(),
            description: latest.description.clone(),
            version: latest.version.clone(),
            app_version: latest.app_version.clone().unwrap_or_default(),
            versions: chart.versions.len(),
            initial: chart
                .name
                .chars()
                .next()
                .map_or('?', |c| c.to_ascii_uppercase()),
            hue: (hash % 360) as u16,
        }
    }
}

/// The count line under the title: `12 charts` / `1 chart`.
pub fn count_line(charts: usize, repos: usize) -> String {
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    format!(
        "{} in {}",
        plural(charts, "chart", "charts"),
        plural(repos, "repository", "repositories")
    )
}
