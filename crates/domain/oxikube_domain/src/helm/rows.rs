//! The columns of the Releases table and their order (E16-S05). Pure: the view and the tests
//! sort with the same function.

use std::cmp::Ordering;

use super::catalog::compare_versions;
use super::release::Release;

/// A column of the Releases table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReleaseColumn {
    /// The release name.
    Name,
    /// The release namespace.
    Namespace,
    /// The chart name.
    Chart,
    /// The revision number.
    Revision,
    /// The chart version.
    Version,
    /// The application version the chart packages.
    AppVersion,
    /// The release status.
    Status,
    /// When the revision was last updated.
    Updated,
}

impl ReleaseColumn {
    /// Every column, left to right.
    pub const ALL: [ReleaseColumn; 8] = [
        Self::Name,
        Self::Namespace,
        Self::Chart,
        Self::Revision,
        Self::Version,
        Self::AppVersion,
        Self::Status,
        Self::Updated,
    ];

    /// The header text.
    pub const fn title(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Namespace => "Namespace",
            Self::Chart => "Chart",
            Self::Revision => "Revision",
            Self::Version => "Version",
            Self::AppVersion => "App version",
            Self::Status => "Status",
            Self::Updated => "Updated",
        }
    }

    /// Orders two releases by this column. Ties fall back to namespace and name, so a sort is
    /// deterministic.
    pub fn compare(self, a: &Release, b: &Release) -> Ordering {
        let by_column = match self {
            Self::Name => cmp_text(&a.release.name, &b.release.name),
            Self::Namespace => cmp_text(&a.release.namespace, &b.release.namespace),
            Self::Chart => cmp_text(&a.chart.name, &b.chart.name),
            Self::Revision => a.revision.cmp(&b.revision),
            Self::Version => compare_versions(&a.chart.version, &b.chart.version),
            Self::AppVersion => compare_versions(
                a.app_version.as_deref().unwrap_or_default(),
                b.app_version.as_deref().unwrap_or_default(),
            ),
            Self::Status => a.status.as_str().cmp(b.status.as_str()),
            Self::Updated => a.updated.cmp(&b.updated),
        };
        by_column
            .then_with(|| cmp_text(&a.release.namespace, &b.release.namespace))
            .then_with(|| cmp_text(&a.release.name, &b.release.name))
    }
}

fn cmp_text(a: &str, b: &str) -> Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| a.cmp(b))
}

/// Sorts `rows` by `column`. Stable and deterministic.
pub fn sort_releases(rows: &mut [Release], column: ReleaseColumn, descending: bool) {
    rows.sort_by(|a, b| {
        let order = column.compare(a, b);
        if descending { order.reverse() } else { order }
    });
}
