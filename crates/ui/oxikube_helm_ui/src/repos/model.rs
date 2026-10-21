//! The page as plain data.

use jiff::Timestamp;
use oxikube_app::{RepoActivity, RepoSnapshot, RepoStatus, helm_repos::RepoSuggestion};

/// How a line is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Plain.
    Neutral,
    /// Worked.
    Good,
    /// In progress.
    Pending,
    /// Failed.
    Error,
    /// De-emphasised.
    Muted,
}

/// One repository row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowModel {
    /// The alias.
    pub name: String,
    /// The index URL.
    pub url: String,
    /// The status line (`Updated 5 min ago`, `Failed: ...`).
    pub status: String,
    /// The status line's colour.
    pub tone: Tone,
    /// This row's refresh is running.
    pub updating: bool,
}

/// Everything the frame shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReposModel {
    /// `helm` is usable. When `false` the page shows guidance instead of the list.
    pub cli_available: bool,
    /// The list has been read.
    pub loaded: bool,
    /// Whether `helm` is usable is known (the first read finished).
    pub checked: bool,
    /// The repositories.
    pub rows: Vec<RowModel>,
    /// An operation is running (the buttons wait).
    pub busy: bool,
    /// What is running, one line.
    pub progress: Option<String>,
    /// The last failure.
    pub problem: Option<String>,
    /// The last success.
    pub notice: Option<String>,
    /// One-click additions.
    pub suggestions: Vec<RepoSuggestion>,
}

impl ReposModel {
    /// The model of `snapshot` at time `now`.
    pub fn build(snapshot: &RepoSnapshot, now: Timestamp) -> Self {
        let entries: Vec<_> = snapshot.rows.iter().map(|r| r.entry.clone()).collect();
        Self {
            cli_available: snapshot.cli_available,
            loaded: snapshot.loaded,
            checked: snapshot.checked,
            rows: snapshot
                .rows
                .iter()
                .map(|row| {
                    let (status, tone) = status_line(&row.status, now);
                    RowModel {
                        name: row.entry.name.clone(),
                        url: row.entry.url.clone(),
                        status,
                        tone,
                        updating: row.status == RepoStatus::Updating,
                    }
                })
                .collect(),
            busy: snapshot.is_busy(),
            progress: progress_line(&snapshot.activity),
            problem: snapshot.problem.clone(),
            notice: snapshot.notice.clone(),
            suggestions: oxikube_app::helm_repos::default_suggestions(&entries),
        }
    }
}

fn progress_line(activity: &RepoActivity) -> Option<String> {
    match activity {
        RepoActivity::Idle => None,
        RepoActivity::Loading => Some("Reading Helm's repositories...".into()),
        RepoActivity::Adding(name) => Some(format!("Adding {name} and fetching its index...")),
        RepoActivity::Removing(name) => Some(format!("Removing {name}...")),
        RepoActivity::Updating(names) if names.len() == 1 => {
            Some(format!("Updating {}...", names[0]))
        }
        RepoActivity::Updating(names) => Some(format!("Updating {} repositories...", names.len())),
    }
}

fn status_line(status: &RepoStatus, now: Timestamp) -> (String, Tone) {
    match status {
        RepoStatus::Unknown => ("Not refreshed in this session".into(), Tone::Muted),
        RepoStatus::Updating => ("Updating...".into(), Tone::Pending),
        RepoStatus::Updated(at) => (format!("Updated {}", ago(*at, now)), Tone::Good),
        RepoStatus::Failed(why) => (format!("Failed: {why}"), Tone::Error),
    }
}

/// `just now`, `5 min ago`, `3 h ago`, `2 d ago`.
fn ago(at: Timestamp, now: Timestamp) -> String {
    let secs = now.as_second().saturating_sub(at.as_second()).max(0);
    match secs {
        0..=44 => "just now".into(),
        45..=3_599 => format!("{} min ago", (secs + 30) / 60),
        3_600..=86_399 => format!("{} h ago", secs / 3_600),
        _ => format!("{} d ago", secs / 86_400),
    }
}
