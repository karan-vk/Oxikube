//! The charts catalog view-model: search hits grouped by chart, newest version first.

use std::cmp::Ordering;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::repo::{ChartHit, ShowKind};

/// The part of a chart a `helm::ShowChart` command prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChartPart {
    /// `Chart.yaml`.
    Chart,
    /// The README (markdown).
    #[default]
    Readme,
    /// The default `values.yaml`.
    Values,
}

impl ChartPart {
    /// The `helm show` kind that prints this part.
    pub const fn show_kind(self) -> ShowKind {
        match self {
            Self::Chart => ShowKind::Chart,
            Self::Readme => ShowKind::Readme,
            Self::Values => ShowKind::Values,
        }
    }
}

/// One version of a chart in the catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartVersion {
    /// The chart version (`15.4.0`).
    pub version: String,
    /// The application version this chart version packages.
    pub app_version: Option<String>,
    /// The one-line description of this version.
    pub description: String,
}

/// A chart of a repository with all the versions a search found, for a catalog card.
///
/// [`versions`](Self::versions) is never empty and is ordered newest first (semver aware), so
/// [`latest`](Self::latest) is the card's version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartSummary {
    /// The repository alias (`bitnami`), when the search reported one.
    pub repo: Option<String>,
    /// The chart name (`nginx`).
    pub name: String,
    /// Every version found, newest first.
    pub versions: Vec<ChartVersion>,
}

impl ChartSummary {
    /// The newest version: the card's version, app version and description.
    pub fn latest(&self) -> &ChartVersion {
        &self.versions[0]
    }

    /// `repo/name`, or just the name without a repository.
    pub fn reference(&self) -> String {
        match &self.repo {
            Some(repo) => format!("{repo}/{}", self.name),
            None => self.name.clone(),
        }
    }

    /// A chart reference to `version` (the newest when `None`).
    pub fn chart_ref(&self, version: Option<&str>) -> super::release::ChartRef {
        let version = version.unwrap_or(&self.latest().version);
        let chart = super::release::ChartRef::new(&self.name, version);
        match &self.repo {
            Some(repo) => chart.in_repo(repo),
            None => chart,
        }
    }
}

/// Groups search hits by `(repo, name)`, orders each chart's versions newest first and the
/// charts by name then repository. A hit repeated for one version is kept once.
pub fn group_hits(hits: Vec<ChartHit>) -> Vec<ChartSummary> {
    let mut index: HashMap<(Option<String>, String), usize> = HashMap::new();
    let mut charts: Vec<ChartSummary> = Vec::new();
    for hit in hits {
        let key = (hit.chart.repo.clone(), hit.chart.name.clone());
        let version = ChartVersion {
            version: hit.chart.version,
            app_version: hit.app_version,
            description: hit.description,
        };
        match index.get(&key) {
            Some(&i) => {
                if !charts[i]
                    .versions
                    .iter()
                    .any(|v| v.version == version.version)
                {
                    charts[i].versions.push(version);
                }
            }
            None => {
                index.insert(key.clone(), charts.len());
                charts.push(ChartSummary {
                    repo: key.0,
                    name: key.1,
                    versions: vec![version],
                });
            }
        }
    }
    for chart in &mut charts {
        chart
            .versions
            .sort_by(|a, b| compare_versions(&b.version, &a.version));
    }
    charts.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.repo.cmp(&b.repo))
    });
    charts
}

/// Orders two chart versions by semver precedence: numeric `major.minor.patch`, then a
/// pre-release (`1.0.0-rc.1`) before its release, pre-release identifiers compared as semver
/// says. Build metadata (`+x`) and a leading `v` are ignored. A version that is not semver sorts
/// before any that is, and two of those compare as text.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    match (Parsed::new(a), Parsed::new(b)) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => a.cmp(b),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Parsed<'a> {
    core: [u64; 3],
    pre: Option<Vec<&'a str>>,
}

impl<'a> Parsed<'a> {
    fn new(version: &'a str) -> Option<Self> {
        let v = version.trim().trim_start_matches('v');
        let v = v.split('+').next()?;
        let (core, pre) = match v.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (v, None),
        };
        let mut parts = core.split('.');
        let mut nums = [0u64; 3];
        for n in &mut nums {
            *n = match parts.next() {
                Some(p) => p.parse().ok()?,
                None => 0,
            };
        }
        parts.next().is_none().then(|| Self {
            core: nums,
            pre: pre.map(|p| p.split('.').collect()),
        })
    }
}

impl Ord for Parsed<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core
            .cmp(&other.core)
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => cmp_pre(a, b),
            })
    }
}

impl PartialOrd for Parsed<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn cmp_pre(a: &[&str], b: &[&str]) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            (Err(_), Err(_)) => x.cmp(y),
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    a.len().cmp(&b.len())
}
