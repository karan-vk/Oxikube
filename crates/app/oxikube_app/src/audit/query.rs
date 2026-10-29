//! The audit viewer's model: the filters it offers, and reading the log a keyset page at a time.
//!
//! The viewer never loads the whole log. [`AuditFilters`] become an
//! [`AuditQuery`](oxikube_ports::AuditQuery) (so the store's indices answer them), and
//! [`AuditReader::page`] reads one page; the cursor of the page is the position of the next one.
//! [`count_new`] says how many records a fresh head page holds that the viewer has not seen, for
//! the "N new" pill.

use std::sync::Arc;
use std::time::Duration;

use jiff::{Timestamp, ToSpan as _};
use oxikube_domain::OxiResult;
use oxikube_domain::audit::{AuditOutcome, AuditRecord, Initiator};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::safety::MutationVerb;
use oxikube_ports::{AuditCursor, AuditPage, AuditQuery, StatePort};

/// Records read per page: enough to fill a tall window, small enough to answer in a frame or two.
pub const PAGE_SIZE: usize = 200;

/// The most records of one batch the detail panel lists.
pub const MAX_BATCH_RECORDS: usize = 1_000;

/// How far back the viewer looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AuditWindow {
    /// The whole log (within retention).
    #[default]
    All,
    /// The last hour.
    LastHour,
    /// The last 24 hours.
    LastDay,
    /// The last 7 days.
    LastWeek,
}

impl AuditWindow {
    /// Every window, widest first.
    pub const ALL: [AuditWindow; 4] = [
        AuditWindow::All,
        AuditWindow::LastHour,
        AuditWindow::LastDay,
        AuditWindow::LastWeek,
    ];

    /// The name shown on the filter.
    pub const fn label(self) -> &'static str {
        match self {
            AuditWindow::All => "All time",
            AuditWindow::LastHour => "Last hour",
            AuditWindow::LastDay => "Last 24 hours",
            AuditWindow::LastWeek => "Last 7 days",
        }
    }

    /// The earliest time the window includes, as of `now`; `None` for [`All`](Self::All).
    pub fn since(self, now: Timestamp) -> Option<Timestamp> {
        let back = match self {
            AuditWindow::All => return None,
            AuditWindow::LastHour => 1.hour(),
            AuditWindow::LastDay => 24.hours(),
            AuditWindow::LastWeek => 168.hours(),
        };
        now.checked_sub(back).ok()
    }
}

/// What the viewer's filter bar asks for. Every set field must match; the default shows the
/// whole log.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AuditFilters {
    /// Only this cluster's records.
    pub cluster: Option<ClusterId>,
    /// Only records of this verb.
    pub verb: Option<MutationVerb>,
    /// Only records that came through this door.
    pub initiator: Option<Initiator>,
    /// Only records that ended this way.
    pub outcome: Option<AuditOutcome>,
    /// How far back to look.
    pub window: AuditWindow,
}

impl AuditFilters {
    /// Whether no filter is set.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The store query for the page after `cursor` (the newest page when `None`), as of `now`.
    pub fn query(&self, now: Timestamp, cursor: Option<AuditCursor>, limit: usize) -> AuditQuery {
        AuditQuery {
            cluster: self.cluster.clone(),
            since: self.window.since(now),
            verb: self.verb,
            initiator: self.initiator,
            outcome: self.outcome,
            cursor,
            limit,
            ..AuditQuery::default()
        }
    }
}

/// Reads the audit log for the viewer and the export.
#[derive(Clone)]
pub struct AuditReader {
    state: Arc<dyn StatePort>,
}

impl std::fmt::Debug for AuditReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditReader").finish_non_exhaustive()
    }
}

impl AuditReader {
    /// A reader over `state`.
    pub fn new(state: Arc<dyn StatePort>) -> Self {
        Self { state }
    }

    /// The page of records `filters` match after `cursor` (the newest page when `None`).
    ///
    /// # Errors
    ///
    /// The store's error (the database is unavailable, a query failed).
    pub async fn page(
        &self,
        filters: &AuditFilters,
        now: Timestamp,
        cursor: Option<AuditCursor>,
        limit: usize,
    ) -> OxiResult<AuditPage> {
        self.state
            .query_audit(&filters.query(now, cursor, limit))
            .await
    }

    /// The records of one bulk operation, newest first, at most [`MAX_BATCH_RECORDS`].
    ///
    /// # Errors
    ///
    /// The store's error.
    pub async fn batch(&self, batch_id: &str) -> OxiResult<Vec<AuditRecord>> {
        let mut out = Vec::new();
        let mut cursor = None;
        loop {
            let query = AuditQuery {
                batch_id: Some(batch_id.to_owned()),
                cursor,
                limit: PAGE_SIZE.min(MAX_BATCH_RECORDS - out.len()),
                ..AuditQuery::default()
            };
            let page = self.state.query_audit(&query).await?;
            out.extend(page.records);
            cursor = page.next;
            if cursor.is_none() || out.len() >= MAX_BATCH_RECORDS {
                return Ok(out);
            }
        }
    }
}

/// How many records of a fresh head page the viewer has not shown yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewRecords {
    /// Exactly this many.
    Exactly(usize),
    /// At least this many: the whole fresh page is newer than the viewer's head.
    AtLeast(usize),
}

impl NewRecords {
    /// The count, for display (`3`, `200+`).
    pub fn label(self) -> String {
        match self {
            NewRecords::Exactly(n) => n.to_string(),
            NewRecords::AtLeast(n) => format!("{n}+"),
        }
    }

    /// Whether there is nothing new.
    pub fn is_none(self) -> bool {
        self == NewRecords::Exactly(0)
    }
}

/// How many records of `fresh` (the newest page, newest first) come before `head`, the newest
/// record the viewer shows. With no `head` (the viewer was empty) every fresh record is new.
pub fn count_new(head: Option<&AuditRecord>, fresh: &[AuditRecord]) -> NewRecords {
    let Some(head) = head else {
        return NewRecords::Exactly(fresh.len());
    };
    if let Some(n) = fresh.iter().position(|record| record == head) {
        return NewRecords::Exactly(n);
    }
    // The head is not in the page: a sliding time window aged it out, or retention pruned it.
    // Only records strictly newer than it are new, so an empty or older-only page is no news.
    let newer = fresh
        .iter()
        .take_while(|record| record.ts > head.ts)
        .count();
    if newer == fresh.len() && newer > 0 {
        NewRecords::AtLeast(newer)
    } else {
        NewRecords::Exactly(newer)
    }
}

/// How long the viewer waits for the store before it shows an error instead of "Loading…"; the
/// view races each read against a timer of this length.
pub const READ_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(test)]
mod tests;
