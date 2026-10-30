//! [`AuditModel`]: the records the viewer has loaded, newest first, the keyset cursor to read on
//! from, and the selection. Plain data, no GPUI.
//!
//! The model holds at most [`MAX_LOADED_RECORDS`]: a log of hundreds of thousands of records is
//! never all in memory (about 1.4 KB a record), and scrolling to its end stops at the newest
//! [`MAX_LOADED_RECORDS`] with a note to narrow the filters or export the rest. Records stored
//! while the viewer is open go in front and the oldest loaded ones drop off the end, so the bound
//! holds during a bulk operation too, and putting them in front never moves the loaded ones.

use std::collections::{HashMap, VecDeque};

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

/// Most records the viewer holds (see the [module docs](self)): about 14 MiB of records.
pub const MAX_LOADED_RECORDS: usize = 10_000;

/// The loaded records. See the [module docs](self).
#[derive(Debug, Default)]
pub struct AuditModel {
    records: VecDeque<AuditRecord>,
    cursor: Option<AuditCursor>,
    exhausted: bool,
    /// Older records exist that the model will not load: it holds [`MAX_LOADED_RECORDS`].
    capped: bool,
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
    pub fn records(&self) -> &VecDeque<AuditRecord> {
        &self.records
    }

    /// The record at row `ix`.
    pub fn get(&self, ix: usize) -> Option<&AuditRecord> {
        self.records.get(ix)
    }

    /// The newest loaded record.
    pub fn head(&self) -> Option<&AuditRecord> {
        self.records.front()
    }

    /// Whether older records remain to be read.
    pub fn has_more(&self) -> bool {
        !self.capped && !self.exhausted && self.cursor.is_some()
    }

    /// Whether older records exist than the model will load: it holds [`MAX_LOADED_RECORDS`].
    pub fn capped(&self) -> bool {
        self.capped
    }

    /// Where the next page starts.
    pub fn cursor(&self) -> Option<AuditCursor> {
        self.cursor
    }

    /// Replaces everything with the newest page (a new filter, a reload). The selection follows
    /// the selected record when it is still there. Returns the records it replaced so the caller
    /// can free them off the UI thread (tens of thousands of strings).
    #[must_use = "drop the replaced records off the UI thread"]
    pub fn replace(
        &mut self,
        records: Vec<AuditRecord>,
        next: Option<AuditCursor>,
    ) -> VecDeque<AuditRecord> {
        let selected = self.selected_record().cloned();
        let old = std::mem::replace(&mut self.records, VecDeque::from(records));
        self.cursor = next;
        self.exhausted = next.is_none();
        self.capped = false;
        self.cap();
        self.selected = selected.and_then(|s| self.records.iter().position(|r| *r == s));
        old
    }

    /// Adds an older page at the end, up to [`MAX_LOADED_RECORDS`].
    pub fn append(&mut self, records: Vec<AuditRecord>, next: Option<AuditCursor>) {
        self.records.extend(records);
        self.cursor = next;
        self.exhausted = next.is_none();
        self.cap();
        if self.records.len() >= MAX_LOADED_RECORDS && !self.exhausted {
            self.capped = true;
        }
    }

    /// Puts `fresh` (newer than every loaded record, newest first) in front; the selection keeps
    /// its record. The oldest loaded records drop off the end past [`MAX_LOADED_RECORDS`].
    pub fn prepend(&mut self, fresh: Vec<AuditRecord>) {
        if fresh.is_empty() {
            return;
        }
        let added = fresh.len();
        for record in fresh.into_iter().rev() {
            self.records.push_front(record);
        }
        self.selected = self.selected.map(|ix| ix + added);
        self.cap();
    }

    /// Drops the oldest records past [`MAX_LOADED_RECORDS`] (they can no longer be paged to, so
    /// the model is capped) and a selection that pointed at one.
    fn cap(&mut self) {
        if self.records.len() > MAX_LOADED_RECORDS {
            self.records.truncate(MAX_LOADED_RECORDS);
            self.capped = true;
            self.selected = self.selected.filter(|ix| *ix < MAX_LOADED_RECORDS);
        }
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
