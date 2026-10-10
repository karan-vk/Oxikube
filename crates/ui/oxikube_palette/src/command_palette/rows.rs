//! The palette's data: one [`Row`] per registered command with its availability in the context
//! the palette was opened in, the fuzzy candidates, and the order matches are listed in.
//!
//! Everything here is plain data built once when the palette opens (one pass over the command
//! index: 2 000 commands take well under a millisecond) and shared with the matching task by
//! `Arc`, so a keystroke filters off the UI thread without copying the commands.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::SharedString;
use oxikube_app::{CommandContext, CommandIndex, CommandInfo, Unavailable};
use oxikube_domain::command::CommandId;

use crate::picker::fuzzy::{StringMatch, StringMatchCandidate};

/// One command as the palette lists it.
#[derive(Debug, Clone)]
pub struct Row {
    /// The registered command.
    pub info: CommandInfo,
    /// Why it cannot run in the captured context; `None` when it can.
    pub unavailable: Option<Unavailable>,
    /// Where the title starts in the row's match text (`"{category} {title}"`): the bytes before
    /// it are the category chip.
    pub title_offset: usize,
}

impl Row {
    /// The text the query is matched against: the category, then the title, so `pod shell` finds
    /// "Shell" in the Pod category.
    fn match_text(info: &CommandInfo) -> (SharedString, usize) {
        let category = info.category().label();
        let text = format!("{category} {}", info.title());
        (text.into(), category.len() + 1)
    }
}

/// The commands of one palette session: every registered command with its availability, and the
/// fuzzy candidates for the two lists (the default one hides the unavailable).
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Every command, in display order (category, then title). A candidate's id is its index.
    pub rows: Arc<[Row]>,
    /// The candidates of the commands that can run here.
    pub available: Arc<[StringMatchCandidate]>,
    /// The candidates of every command (the "show all" list).
    pub everything: Arc<[StringMatchCandidate]>,
}

impl Snapshot {
    /// Classifies every command of `index` against `context`.
    pub fn take(index: &CommandIndex, context: &CommandContext) -> Self {
        let all = index.all();
        let mut rows = Vec::with_capacity(all.len());
        let mut available = Vec::with_capacity(all.len());
        let mut everything = Vec::with_capacity(all.len());
        for (ix, info) in all.iter().enumerate() {
            let unavailable = info.check(context).err();
            let (text, title_offset) = Row::match_text(info);
            let candidate = StringMatchCandidate::new(ix, text);
            if unavailable.is_none() {
                available.push(candidate.clone());
            }
            everything.push(candidate);
            rows.push(Row {
                info: *info,
                unavailable,
                title_offset,
            });
        }
        Self {
            rows: rows.into(),
            available: available.into(),
            everything: everything.into(),
        }
    }

    /// The candidates of the list to show.
    pub fn candidates(&self, show_all: bool) -> Arc<[StringMatchCandidate]> {
        if show_all {
            self.everything.clone()
        } else {
            self.available.clone()
        }
    }

    /// How many commands the default list hides.
    pub fn hidden(&self) -> usize {
        self.rows.len() - self.available.len()
    }
}

/// A match in the order the palette lists it.
#[derive(Debug, Clone)]
pub struct Found {
    /// Index into [`Snapshot::rows`].
    pub row: usize,
    /// Matched characters, as byte offsets into the row's match text.
    pub positions: Vec<usize>,
}

/// The matches as the palette lists them. [`FuzzyService`](oxikube_app::FuzzyService) has already
/// ordered them: best score first (the recent commands get a small boost), among equal scores the
/// recent commands first (most recent first), then alphabetically. An empty query scores every
/// command alike, so it lists the recent commands first and the rest by category and title.
pub fn into_found(matches: Vec<StringMatch>) -> Vec<Found> {
    matches
        .into_iter()
        .map(|found| Found {
            row: found.candidate_id,
            positions: found.positions,
        })
        .collect()
}

/// The recents boost of a candidate: its place among the recent commands, by the candidate's row.
pub fn recency_of(
    rows: Arc<[Row]>,
    recent: Arc<HashMap<CommandId, usize>>,
) -> impl Fn(usize) -> Option<usize> + Send + 'static {
    move |row| recent.get(&rows[row].info.id()).copied()
}

/// `recent` (most recent first) as a rank per command.
pub fn ranks(recent: &[CommandId]) -> HashMap<CommandId, usize> {
    recent
        .iter()
        .enumerate()
        .map(|(rank, id)| (*id, rank))
        .collect()
}
