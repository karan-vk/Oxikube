//! Loading the catalog: what each entry carries, and the order of the list.

use jiff::Timestamp;
use oxikube_domain::ErrorKind;
use oxikube_domain::OxiError;
use oxikube_ports::SourceId;

use super::{Harness, ctx, id};
use crate::catalog::CatalogEntry;

#[test]
fn entries_carry_name_cluster_user_and_source_file() {
    let h = Harness::new();
    let entries = h.load();
    let names: Vec<_> = entries.iter().map(CatalogEntry::name).collect();
    assert_eq!(names, ["a", "b", "c"], "in the order the source lists them");

    let a = &entries[0];
    assert_eq!(a.id(), &id("a"));
    assert_eq!(a.context.cluster_name.as_deref(), Some("a-cluster"));
    assert_eq!(a.context.user.as_deref(), Some("a-user"));
    assert_eq!(a.source_label(), "~/.kube/config");
    assert_eq!(
        a.source.as_ref().and_then(|s| s.path.as_deref()),
        Some(std::path::Path::new("/home/me/.kube/config"))
    );
    assert!(!a.favourite);
    assert_eq!(a.last_used, None);
}

#[test]
fn an_entry_of_an_unlisted_source_shows_the_source_id() {
    let h = Harness::new();
    let mut orphan = ctx("orphan");
    orphan.source = SourceId("vanished".into());
    h.source.set_contexts([orphan]);
    let entry = &h.load()[0];
    assert_eq!(entry.source, None);
    assert_eq!(entry.source_label(), "vanished");
}

#[test]
fn a_failing_source_list_still_lists_the_contexts() {
    let h = Harness::new();
    h.source
        .script()
        .sources
        .push_err(OxiError::internal("cannot read sources"));
    let entries = h.load();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].source_label(), "default");
}

#[test]
fn a_failing_context_list_is_the_error() {
    let h = Harness::new();
    h.source
        .script()
        .contexts
        .push_err(OxiError::internal("cannot read kubeconfig"));
    let error = h.try_load().expect_err("the contexts are the catalog");
    assert_eq!(error.kind(), ErrorKind::Internal);
}

#[test]
fn an_empty_source_is_an_empty_catalog() {
    let h = Harness::new();
    h.source.set_contexts([]);
    assert!(h.load().is_empty());
}

#[test]
fn a_flagged_context_keeps_its_problem() {
    let h = Harness::new();
    let mut broken = ctx("broken");
    broken.problem = Some("cluster \"gone\" is not defined in the kubeconfig".into());
    h.source.set_contexts([broken]);
    let entry = &h.load()[0];
    assert!(
        entry
            .context
            .problem
            .as_deref()
            .is_some_and(|p| p.contains("gone"))
    );
}

fn entry(name: &str, favourite: bool, last_used: Option<i64>) -> CatalogEntry {
    CatalogEntry {
        context: ctx(name),
        source: None,
        favourite,
        last_used: last_used.map(|s| Timestamp::from_second(s).unwrap()),
    }
}

fn sorted(mut entries: Vec<CatalogEntry>) -> Vec<String> {
    entries.sort_by(CatalogEntry::cmp_default);
    entries.iter().map(|e| e.name().to_owned()).collect()
}

#[test]
fn favourites_come_first_then_last_used_then_name() {
    let order = sorted(vec![
        entry("zeta", false, None),
        entry("alpha", false, None),
        entry("old", false, Some(100)),
        entry("recent", false, Some(900)),
        entry("fav-never", true, None),
        entry("fav-used", true, Some(500)),
    ]);
    assert_eq!(
        order,
        ["fav-used", "fav-never", "recent", "old", "alpha", "zeta"]
    );
}

#[test]
fn names_sort_case_insensitively_and_the_order_is_total() {
    let order = sorted(vec![
        entry("beta", false, None),
        entry("Alpha", false, None),
        entry("alpha", false, None),
    ]);
    assert_eq!(order, ["Alpha", "alpha", "beta"]);
}
