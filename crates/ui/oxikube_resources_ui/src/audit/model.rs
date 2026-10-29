//! [`AuditModel`]: the records the viewer has loaded, newest first, the keyset cursor to read on
//! from, and the selection. Plain data, no GPUI.

use std::collections::HashMap;

use gpui::SharedString;
pub use oxikube_app::audit::export::initiator_text;
use oxikube_app::audit::export::outcome_text;
use oxikube_domain::audit::AuditRecord;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::AuditCursor;

/// The table's columns, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuditColumn {
    /// When the attempt ended (UTC).
    Time,
    /// The cluster.
    Cluster,
    /// The namespace.
    Namespace,
    /// What the mutation did.
    Verb,
    /// The kind of the target.
    Kind,
    /// The name of the target.
    Name,
    /// How it ended.
    Outcome,
    /// The door it came through.
    Initiator,
    /// Whether it was a server dry run.
    DryRun,
    /// The acting identity.
    Who,
}

impl AuditColumn {
    /// Every column, in display order.
    pub const ALL: [AuditColumn; 10] = [
        AuditColumn::Time,
        AuditColumn::Cluster,
        AuditColumn::Namespace,
        AuditColumn::Verb,
        AuditColumn::Kind,
        AuditColumn::Name,
        AuditColumn::Outcome,
        AuditColumn::Initiator,
        AuditColumn::DryRun,
        AuditColumn::Who,
    ];

    /// The column at `ix`.
    pub fn at(ix: usize) -> Option<Self> {
        Self::ALL.get(ix).copied()
    }

    /// The stable key of the column.
    pub const fn key(self) -> &'static str {
        match self {
            AuditColumn::Time => "time",
            AuditColumn::Cluster => "cluster",
            AuditColumn::Namespace => "namespace",
            AuditColumn::Verb => "verb",
            AuditColumn::Kind => "kind",
            AuditColumn::Name => "name",
            AuditColumn::Outcome => "outcome",
            AuditColumn::Initiator => "initiator",
            AuditColumn::DryRun => "dry_run",
            AuditColumn::Who => "who",
        }
    }

    /// The header.
    pub const fn title(self) -> &'static str {
        match self {
            AuditColumn::Time => "Time (UTC)",
            AuditColumn::Cluster => "Cluster",
            AuditColumn::Namespace => "Namespace",
            AuditColumn::Verb => "Verb",
            AuditColumn::Kind => "Kind",
            AuditColumn::Name => "Name",
            AuditColumn::Outcome => "Outcome",
            AuditColumn::Initiator => "Initiator",
            AuditColumn::DryRun => "Dry-run",
            AuditColumn::Who => "Who",
        }
    }

    /// The starting width, in logical pixels.
    pub const fn width(self) -> f32 {
        match self {
            AuditColumn::Time => 165.,
            AuditColumn::Cluster | AuditColumn::Namespace => 120.,
            AuditColumn::Verb | AuditColumn::Initiator => 86.,
            AuditColumn::Outcome => 100.,
            AuditColumn::Kind => 110.,
            AuditColumn::Name => 190.,
            AuditColumn::DryRun => 70.,
            AuditColumn::Who => 110.,
        }
    }
}

/// How a cluster is named in the viewer: its context name when a session knows it, else the start
/// of its id.
#[derive(Debug, Clone, Default)]
pub struct ClusterNames(HashMap<ClusterId, SharedString>);

impl ClusterNames {
    /// Names `names`.
    pub fn new(names: impl IntoIterator<Item = (ClusterId, SharedString)>) -> Self {
        Self(names.into_iter().collect())
    }

    /// The name of `cluster`.
    pub fn name(&self, cluster: &ClusterId) -> SharedString {
        self.0.get(cluster).cloned().unwrap_or_else(|| {
            let id = cluster.to_string();
            SharedString::from(id.chars().take(8).collect::<String>())
        })
    }

    /// The known clusters, by name.
    pub fn known(&self) -> Vec<(ClusterId, SharedString)> {
        let mut known: Vec<_> = self
            .0
            .iter()
            .map(|(id, name)| (id.clone(), name.clone()))
            .collect();
        known.sort_by(|a, b| a.1.cmp(&b.1));
        known
    }
}

/// The text of `column` for `record`.
pub fn cell_text(record: &AuditRecord, column: AuditColumn, names: &ClusterNames) -> SharedString {
    match column {
        AuditColumn::Time => record.ts.strftime("%Y-%m-%d %H:%M:%S").to_string().into(),
        AuditColumn::Cluster => names.name(&record.cluster),
        AuditColumn::Namespace => record
            .target
            .namespace
            .as_deref()
            .unwrap_or("-")
            .to_owned()
            .into(),
        AuditColumn::Verb => record.verb.map_or("-", |verb| verb.as_str()).into(),
        AuditColumn::Kind => record.target.gvk.kind.to_string().into(),
        AuditColumn::Name => record.target.name.to_string().into(),
        AuditColumn::Outcome => outcome_text(record.outcome).into(),
        AuditColumn::Initiator => initiator_text(record.initiator).into(),
        AuditColumn::DryRun => if record.dry_run { "yes" } else { "no" }.into(),
        AuditColumn::Who => record.who.to_string().into(),
    }
}

/// The loaded records. See the [module docs](self).
#[derive(Debug, Default)]
pub struct AuditModel {
    records: Vec<AuditRecord>,
    cursor: Option<AuditCursor>,
    exhausted: bool,
    selected: Option<usize>,
}

impl AuditModel {
    /// No records.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many records are loaded.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether nothing is loaded.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The loaded records, newest first.
    pub fn records(&self) -> &[AuditRecord] {
        &self.records
    }

    /// The record at row `ix`.
    pub fn get(&self, ix: usize) -> Option<&AuditRecord> {
        self.records.get(ix)
    }

    /// The newest loaded record.
    pub fn head(&self) -> Option<&AuditRecord> {
        self.records.first()
    }

    /// Whether older records remain to be read.
    pub fn has_more(&self) -> bool {
        !self.exhausted && self.cursor.is_some()
    }

    /// Where the next page starts.
    pub fn cursor(&self) -> Option<AuditCursor> {
        self.cursor
    }

    /// Replaces everything with the newest page (a new filter, a reload). The selection follows
    /// the selected record when it is still there.
    pub fn replace(&mut self, records: Vec<AuditRecord>, next: Option<AuditCursor>) {
        let selected = self.selected_record().cloned();
        self.records = records;
        self.cursor = next;
        self.exhausted = next.is_none();
        self.selected = selected.and_then(|s| self.records.iter().position(|r| *r == s));
    }

    /// Adds an older page at the end.
    pub fn append(&mut self, records: Vec<AuditRecord>, next: Option<AuditCursor>) {
        self.records.extend(records);
        self.cursor = next;
        self.exhausted = next.is_none();
    }

    /// Puts `fresh` (newer than every loaded record, newest first) in front; the selection keeps
    /// its record.
    pub fn prepend(&mut self, mut fresh: Vec<AuditRecord>) {
        if fresh.is_empty() {
            return;
        }
        let added = fresh.len();
        fresh.append(&mut self.records);
        self.records = fresh;
        self.selected = self.selected.map(|ix| ix + added);
    }

    /// Selects row `ix` (`None` clears the selection).
    pub fn select(&mut self, ix: Option<usize>) -> bool {
        let ix = ix.filter(|ix| *ix < self.records.len());
        let changed = self.selected != ix;
        self.selected = ix;
        changed
    }

    /// The selected row.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The selected record.
    pub fn selected_record(&self) -> Option<&AuditRecord> {
        self.selected.and_then(|ix| self.records.get(ix))
    }

    /// Moves the selection `delta` rows (from the first row when there is none).
    pub fn step(&mut self, delta: isize) -> Option<usize> {
        if self.records.is_empty() {
            return None;
        }
        let last = self.records.len() - 1;
        let ix = match self.selected {
            Some(ix) => ix.saturating_add_signed(delta).min(last),
            None => 0,
        };
        self.selected = Some(ix);
        Some(ix)
    }
}
