//! The count fallback (E13-S10): one metadata request with `limit=1` per namespace, the
//! remaining count added, a lower bound said as such, a forbidden kind an error and not zero.

use std::collections::BTreeSet;

use futures::executor::block_on;
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::{ErrorKind, ObjectMeta, OxiError};
use oxikube_ports::ListPage;
use oxikube_testkit::{FakeResourcePort, ResourceCall};

use crate::store::{CountTarget, FetchedCount, fetch_count};

fn pods() -> CountTarget {
    CountTarget::core("", "pods").expect("a core target")
}

fn nodes() -> CountTarget {
    CountTarget::core("", "nodes").expect("a core target")
}

fn meta(name: &str) -> ObjectMeta {
    ObjectMeta::named(name)
}

fn page(held: usize, token: Option<&str>, remaining: Option<i64>) -> ListPage<ObjectMeta> {
    ListPage {
        items: (0..held).map(|i| meta(&format!("o{i}"))).collect(),
        continue_token: token.map(str::to_owned),
        resource_version: None,
        remaining_item_count: remaining,
    }
}

#[test]
fn the_remaining_count_is_added_to_the_page() {
    let reader = FakeResourcePort::new();
    reader
        .script()
        .list_metadata
        .push_ok(page(1, Some("next"), Some(9_999)));
    let found = block_on(fetch_count(&reader, &pods(), &NamespaceSelection::All)).expect("a count");
    assert_eq!(
        found,
        FetchedCount {
            total: 10_000,
            exact: true
        }
    );
    // One metadata request with a page of one, never a list of bodies.
    let calls = reader.recorded_calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    let ResourceCall::ListMetadata {
        namespace, options, ..
    } = &calls[0]
    else {
        panic!("{calls:?}");
    };
    assert_eq!((namespace.as_deref(), options.limit), (None, Some(1)));
}

#[test]
fn a_single_page_is_exact() {
    let reader = FakeResourcePort::new();
    reader.script().list_metadata.push_ok(page(0, None, None));
    let found =
        block_on(fetch_count(&reader, &nodes(), &NamespaceSelection::All)).expect("a count");
    assert_eq!((found.total, found.exact), (0, true));
}

#[test]
fn a_server_that_does_not_say_how_many_remain_gives_a_lower_bound() {
    let reader = FakeResourcePort::new();
    reader
        .script()
        .list_metadata
        .push_ok(page(1, Some("next"), None));
    let found = block_on(fetch_count(&reader, &pods(), &NamespaceSelection::All)).expect("a count");
    assert_eq!((found.total, found.exact), (2, false), "at least two");
}

#[test]
fn a_namespace_selection_asks_once_per_namespace_and_sums() {
    let reader = FakeResourcePort::new();
    reader
        .script()
        .list_metadata
        .push_ok(page(1, Some("a"), Some(4)))
        .push_ok(page(1, None, None));
    let selection = NamespaceSelection::Set(BTreeSet::from(["dev".to_owned(), "prod".to_owned()]));
    let found = block_on(fetch_count(&reader, &pods(), &selection)).expect("a count");
    assert_eq!(found.total, 6);
    let asked: Vec<Option<String>> = reader
        .recorded_calls()
        .into_iter()
        .filter_map(|call| match call {
            ResourceCall::ListMetadata { namespace, .. } => Some(namespace),
            _ => None,
        })
        .collect();
    assert_eq!(asked, [Some("dev".to_owned()), Some("prod".to_owned())]);
}

#[test]
fn a_cluster_scoped_kind_ignores_the_namespace_selection() {
    let reader = FakeResourcePort::new();
    reader.script().list_metadata.push_ok(page(1, None, None));
    let selection = NamespaceSelection::Set(BTreeSet::from(["dev".to_owned()]));
    block_on(fetch_count(&reader, &nodes(), &selection)).expect("a count");
    let calls = reader.recorded_calls();
    assert!(
        matches!(
            &calls[..],
            [ResourceCall::ListMetadata {
                namespace: None,
                ..
            }]
        ),
        "{calls:?}"
    );
}

#[test]
fn a_forbidden_kind_is_an_error_not_zero() {
    let reader = FakeResourcePort::new();
    reader
        .script()
        .list_metadata
        .push_err(OxiError::forbidden("pods is forbidden"));
    let error =
        block_on(fetch_count(&reader, &pods(), &NamespaceSelection::All)).expect_err("forbidden");
    assert_eq!(error.kind(), ErrorKind::Forbidden);
}
