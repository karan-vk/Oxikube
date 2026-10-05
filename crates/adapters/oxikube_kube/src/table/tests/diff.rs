//! Row identity and refresh diffing: only changed rows are re-sent, replays are dropped.

use oxikube_domain::ObjectMeta;
use oxikube_ports::{Delta, TableRow};
use serde_json::json;

use crate::table::index::{Relist, RowIndex, RowKey};

fn row(name: &str, rv: &str) -> TableRow {
    let mut meta = ObjectMeta::named(name);
    meta.namespace = Some("ns".into());
    meta.uid = Some(format!("uid-{name}").into());
    meta.resource_version = Some(rv.into());
    TableRow {
        cells: vec![json!(name), json!(rv)],
        meta: Some(meta),
        object: None,
    }
}

fn label(delta: &Delta<TableRow>) -> String {
    let (what, row) = match delta {
        Delta::Applied(row) => ("applied", row),
        Delta::Deleted(row) => ("deleted", row),
        Delta::Restarted(_) => return "restarted".into(),
    };
    let meta = row.meta.as_ref().unwrap();
    format!(
        "{what} {}@{}",
        meta.name,
        meta.resource_version.as_deref().unwrap_or("-")
    )
}

fn relist(index: &mut RowIndex, pages: Vec<Vec<TableRow>>) -> Vec<String> {
    let mut diff = Relist::default();
    for page in pages {
        diff.add(index, page);
    }
    diff.finish(index).iter().map(label).collect()
}

#[test]
fn a_refresh_sends_only_new_changed_and_deleted_rows() {
    let mut index = RowIndex::default();
    index.reset(&[row("a", "1"), row("b", "1"), row("c", "1")]);
    let deltas = relist(
        &mut index,
        vec![vec![row("a", "1"), row("b", "2")], vec![row("d", "1")]],
    );
    assert_eq!(deltas, ["applied b@2", "applied d@1", "deleted c@1"]);
    assert_eq!(index.len(), 3);
    // The next identical refresh sends nothing.
    let again = relist(
        &mut index,
        vec![vec![row("a", "1"), row("b", "2"), row("d", "1")]],
    );
    assert!(again.is_empty(), "{again:?}");
}

#[test]
fn a_deleted_row_from_a_refresh_carries_its_last_metadata_and_no_cells() {
    let mut index = RowIndex::default();
    index.reset(&[row("gone", "7")]);
    let mut diff = Relist::default();
    diff.add(&index, vec![]);
    let deltas = diff.finish(&mut index);
    let Delta::Deleted(deleted) = &deltas[0] else {
        panic!("expected a deletion");
    };
    assert!(deleted.cells.is_empty());
    assert_eq!(
        deleted.meta.as_ref().unwrap().resource_version.as_deref(),
        Some("7")
    );
    assert_eq!(index.len(), 0);
}

#[test]
fn a_replayed_watch_event_is_dropped_and_a_new_version_applies() {
    let mut index = RowIndex::default();
    index.reset(&[row("a", "5")]);
    assert!(
        index.apply(row("a", "5")).is_none(),
        "same version is a replay"
    );
    assert_eq!(
        index.apply(row("a", "6")).as_ref().map(label).as_deref(),
        Some("applied a@6")
    );
    assert_eq!(label(&index.remove(row("a", "7"))), "deleted a@7");
    assert_eq!(index.len(), 0);
}

#[test]
fn rows_without_a_version_are_always_resent() {
    let mut index = RowIndex::default();
    let mut unversioned = row("a", "1");
    unversioned.meta.as_mut().unwrap().resource_version = None;
    index.reset(std::slice::from_ref(&unversioned));
    assert!(index.apply(unversioned.clone()).is_some());
    let mut diff = Relist::default();
    diff.add(&index, vec![unversioned]);
    assert_eq!(diff.finish(&mut index).len(), 1);
}

#[test]
fn rows_are_keyed_by_uid_then_namespace_and_name() {
    let with_uid = row("a", "1");
    assert_eq!(
        RowKey::of(with_uid.meta.as_ref().unwrap()),
        RowKey::Uid("uid-a".into())
    );
    let mut recreated = row("a", "9");
    recreated.meta.as_mut().unwrap().uid = Some("uid-a-2".into());
    let mut index = RowIndex::default();
    index.reset(&[with_uid]);
    // Same name, new UID: the old object is gone and a new one exists.
    let deltas = relist(&mut index, vec![vec![recreated]]);
    assert_eq!(deltas, ["applied a@9", "deleted a@1"]);

    let mut no_uid = ObjectMeta::named("b");
    no_uid.namespace = Some("ns".into());
    assert_eq!(
        RowKey::of(&no_uid),
        RowKey::Name(Some("ns".into()), "b".into())
    );
}
