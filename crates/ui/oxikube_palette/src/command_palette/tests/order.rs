//! The order of matches: recents first on an empty query, score first on a typed one.

use oxikube_app::{CommandContext, Selection};
use oxikube_domain::Capabilities;
use oxikube_domain::command::{CommandId, ViewContext};

use super::{declared_index, pod};
use crate::command_palette::rows::{Snapshot, order, ranks};
use crate::picker::fuzzy::match_strings;

fn snapshot() -> Snapshot {
    let mut context = CommandContext::new(ViewContext::Table)
        .with_capabilities(Capabilities::all())
        .selecting(Selection::one(pod("web").gvk));
    context.cluster_active = true;
    Snapshot::take(&declared_index(), &context)
}

fn listed(snapshot: &Snapshot, query: &str, recent: &[CommandId]) -> Vec<CommandId> {
    let matches = match_strings(&snapshot.available, query, usize::MAX);
    order(matches, &snapshot.rows, &ranks(recent))
        .into_iter()
        .map(|found| snapshot.rows[found.row].info.id())
        .collect()
}

#[test]
fn an_empty_query_lists_recents_first_then_category_and_title() {
    let snapshot = snapshot();
    let plain = listed(&snapshot, "", &[]);
    let recent = [CommandId::VIEW_ZOOM_OUT, CommandId::POD_VIEW_LOGS];
    let with_recents = listed(&snapshot, "", &recent);
    assert_eq!(with_recents[..2], recent, "most recent first");
    let rest: Vec<_> = with_recents[2..].to_vec();
    let expected: Vec<_> = plain
        .into_iter()
        .filter(|id| !recent.contains(id))
        .collect();
    assert_eq!(rest, expected, "the others keep the display order");
}

#[test]
fn a_typed_query_ranks_by_score_and_breaks_ties_by_recency() {
    let snapshot = snapshot();
    // An exact title beats a recent partial match.
    let found = listed(&snapshot, "zoom out", &[CommandId::VIEW_ZOOM_IN]);
    assert_eq!(found[0], CommandId::VIEW_ZOOM_OUT);

    // Equal scores: the recent one is first. With nothing else to tell them apart (the empty
    // query scores every command 0), recency decides.
    let none = listed(&snapshot, "", &[]);
    let last = *none.last().unwrap();
    let bumped = listed(&snapshot, "", &[last]);
    assert_eq!(bumped[0], last);
}

#[test]
fn the_snapshot_separates_what_can_run_from_what_cannot() {
    let snapshot = snapshot();
    assert!(snapshot.available.len() < snapshot.everything.len());
    assert_eq!(
        snapshot.hidden(),
        snapshot.everything.len() - snapshot.available.len()
    );
    let unavailable = snapshot
        .rows
        .iter()
        .filter(|row| row.unavailable.is_some())
        .count();
    assert_eq!(unavailable, snapshot.hidden());
    // The query matches "category title".
    let found = listed(&snapshot, "pod logs", &[]);
    assert!(found.contains(&CommandId::POD_VIEW_LOGS));
}
