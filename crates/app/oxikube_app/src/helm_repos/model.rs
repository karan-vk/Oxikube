//! What the repositories page shows: plain data.

use jiff::Timestamp;
use oxikube_domain::helm::RepoEntry;

/// How a repository's last refresh went in this session.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RepoStatus {
    /// Not refreshed since Oxikube started (Helm's cache may still be fresh).
    #[default]
    Unknown,
    /// A refresh is running.
    Updating,
    /// Refreshed (or just added) at this time.
    Updated(Timestamp),
    /// The refresh failed; the reason, readable.
    Failed(String),
}

/// One configured repository and its status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRow {
    /// The alias and URL, as Helm lists them.
    pub entry: RepoEntry,
    /// The last refresh.
    pub status: RepoStatus,
}

/// What the service is doing right now.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RepoActivity {
    /// Nothing.
    #[default]
    Idle,
    /// Reading the list.
    Loading,
    /// Adding this repository.
    Adding(String),
    /// Removing this repository.
    Removing(String),
    /// Refreshing these repositories.
    Updating(Vec<String>),
}

/// The page's whole state. Cheap to clone: a handful of short strings.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoSnapshot {
    /// The `helm` binary is available (version gate included); without it nothing here works.
    pub cli_available: bool,
    /// The list has been read at least once.
    pub loaded: bool,
    /// The first read has finished, well or not: whether `helm` is usable is known now. Until
    /// then `cli_available` only means "not detected yet".
    pub checked: bool,
    /// The repositories, in Helm's order.
    pub rows: Vec<RepoRow>,
    /// What is running.
    pub activity: RepoActivity,
    /// The last operation's failure, readable; cleared when the next operation starts.
    pub problem: Option<String>,
    /// The last operation's success, one line; cleared when the next operation starts.
    pub notice: Option<String>,
}

impl RepoSnapshot {
    /// `true` while an operation runs.
    pub fn is_busy(&self) -> bool {
        self.activity != RepoActivity::Idle
    }

    /// The row called `name`.
    pub fn row(&self, name: &str) -> Option<&RepoRow> {
        self.rows.iter().find(|r| r.entry.name == name)
    }
}

/// How a `helm::RepoUpdate` went, per repository.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpdateReport {
    /// The repositories that refreshed.
    pub updated: Vec<String>,
    /// The ones that did not, with the readable reason.
    pub failed: Vec<(String, String)>,
}

impl UpdateReport {
    /// One line: `Updated 3 repositories` / `Updated 2, failed 1 (bitnami: ...)`.
    pub fn summary(&self) -> String {
        let noun = |n: usize| if n == 1 { "repository" } else { "repositories" };
        match (self.updated.len(), self.failed.as_slice()) {
            (0, []) => "No repositories to update".to_owned(),
            (n, []) => format!("Updated {n} {}", noun(n)),
            (0, failed) => format!("Could not update {}", failed_list(failed)),
            (n, failed) => format!("Updated {n}, failed {}", failed_list(failed)),
        }
    }
}

fn failed_list(failed: &[(String, String)]) -> String {
    failed
        .iter()
        .map(|(name, why)| format!("{name} ({why})"))
        .collect::<Vec<_>>()
        .join(", ")
}
