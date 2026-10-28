//! The page as plain data: tabs, load states, how a row reads.

use jiff::Timestamp;
use oxikube_domain::helm::{Release, ReleaseColumn, ReleaseStatus};

/// A tab of the release detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    /// The release's objects with their health.
    Resources,
    /// User-supplied or computed values.
    Values,
    /// The rendered manifest.
    Manifest,
    /// The chart's `NOTES.txt`.
    Notes,
    /// Every revision.
    History,
}

impl Tab {
    /// Every tab, in display order.
    pub const ALL: [Tab; 5] = [
        Tab::Resources,
        Tab::Values,
        Tab::Manifest,
        Tab::Notes,
        Tab::History,
    ];

    /// The tab's label.
    pub const fn title(self) -> &'static str {
        match self {
            Tab::Resources => "Resources",
            Tab::Values => "Values",
            Tab::Manifest => "Manifest",
            Tab::Notes => "Notes",
            Tab::History => "History",
        }
    }

    /// A stable lower-case key (for element ids).
    pub const fn key(self) -> &'static str {
        match self {
            Tab::Resources => "resources",
            Tab::Values => "values",
            Tab::Manifest => "manifest",
            Tab::Notes => "notes",
            Tab::History => "history",
        }
    }

    /// The tab after this one (wrapping).
    #[must_use]
    pub fn next(self) -> Tab {
        Self::ALL[(self as usize + 1) % Self::ALL.len()]
    }

    /// The tab before this one (wrapping).
    #[must_use]
    pub fn previous(self) -> Tab {
        Self::ALL[(self as usize + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// Which values the Values tab shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ValuesMode {
    /// What the user supplied (`helm get values`).
    #[default]
    User,
    /// The chart defaults merged with the user's (`helm get values --all`).
    All,
}

impl ValuesMode {
    /// `true` for [`All`](Self::All).
    pub const fn all(self) -> bool {
        matches!(self, Self::All)
    }

    /// The other mode.
    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::User => Self::All,
            Self::All => Self::User,
        }
    }
}

/// How far a piece of the page is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Load {
    /// Not asked for yet (its tab was not opened).
    #[default]
    Idle,
    /// Being read: the tab shows a skeleton.
    Loading,
    /// Read.
    Ready,
    /// Reading failed; the message is for the user.
    Failed(String),
}

/// Where the list is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// The first read is under way: the table shows its skeleton.
    Loading,
    /// The releases are shown.
    Ready,
    /// Reading failed (no connection, no permission); the message says what to do.
    Failed(String),
}

/// What a status looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Live and well.
    Good,
    /// An operation is under way.
    Busy,
    /// The last operation failed.
    Bad,
    /// History: replaced or removed.
    Quiet,
}

/// The tone of a status (drawn in the theme's success, warning, error and muted colours).
pub const fn tone(status: ReleaseStatus) -> Tone {
    match status {
        ReleaseStatus::Deployed => Tone::Good,
        ReleaseStatus::Failed => Tone::Bad,
        ReleaseStatus::PendingInstall
        | ReleaseStatus::PendingUpgrade
        | ReleaseStatus::PendingRollback
        | ReleaseStatus::Uninstalling => Tone::Busy,
        ReleaseStatus::Superseded | ReleaseStatus::Uninstalled | ReleaseStatus::Unknown => {
            Tone::Quiet
        }
    }
}

/// `3d`, `5h`, `12m`, `40s`: how long ago, as the resource tables write ages.
pub fn age(updated: Timestamp, now: Timestamp) -> String {
    let secs = now.as_second().saturating_sub(updated.as_second()).max(0);
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        86_400..=63_071_999 => format!("{}d", secs / 86_400),
        _ => format!("{}y", secs / 31_536_000),
    }
}

/// The cell texts of one row, built once per refresh (the table draws them as they are).
#[derive(Debug, Clone)]
pub struct RowText {
    cells: [gpui::SharedString; 8],
}

impl RowText {
    /// The texts of `release`, ages measured at `now`.
    pub fn new(release: &Release, now: Timestamp) -> Self {
        let text = |column: ReleaseColumn| -> gpui::SharedString {
            match column {
                ReleaseColumn::Name => release.release.name.clone().into(),
                ReleaseColumn::Namespace => release.release.namespace.clone().into(),
                ReleaseColumn::Chart => release.chart.name.clone().into(),
                ReleaseColumn::Revision => release.revision.to_string().into(),
                ReleaseColumn::Version => release.chart.version.clone().into(),
                ReleaseColumn::AppVersion => release.app_version.clone().unwrap_or_default().into(),
                ReleaseColumn::Status => release.status.as_str().into(),
                ReleaseColumn::Updated => age(release.updated, now).into(),
            }
        };
        Self {
            cells: ReleaseColumn::ALL.map(text),
        }
    }

    /// The text of `column`.
    pub fn cell(&self, column: ReleaseColumn) -> &gpui::SharedString {
        &self.cells[column as usize]
    }
}

/// The links in `notes` that may be opened (`http` and `https`), in order, without repeats, at
/// most `max`. A link ends at whitespace or a closing bracket or quote.
pub fn links(notes: &str, max: usize) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for word in notes.split_whitespace() {
        let Some(start) = word.find("http://").or_else(|| word.find("https://")) else {
            continue;
        };
        let url = word[start..].trim_end_matches(['.', ',', ';', ')', ']', '>', '"', '\'', '`']);
        if oxikube_ui::markdown::is_safe_link(url) && !found.iter().any(|f| f == url) {
            found.push(url.to_owned());
            if found.len() == max {
                break;
            }
        }
    }
    found
}
