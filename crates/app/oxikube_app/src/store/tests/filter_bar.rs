//! The `/` filter on a live subscription (E07-S04): `/foo`, `/!foo`, `/-l app=x` (re-keys the
//! feed with the selector), `/-f` fuzzy ordering, namespace scoping, the total, and incremental
//! narrowing agreeing with a fresh filter.

use std::time::Duration;

use futures::StreamExt;

use oxikube_domain::session::WatchScope;
use oxikube_ports::{Delta, WatchOptions};
use oxikube_testkit::{ResourceCall, Timeline};
use proptest::prelude::*;

use super::*;
use crate::store::filter::parse;
use crate::store::{FeedState, FilterParts, LabelSelector, SortField, SortKey};

fn labelled(ns: &str, name: &str, app: &str) -> Resource {
    let mut r = pod().namespace(ns).name(name).label("app", app).build();
    r.meta.resource_version = Some("1".into());
    r
}

fn fixtures() -> Vec<Resource> {
    vec![
        labelled("x", "web-1", "web"),
        labelled("x", "web-2", "web"),
        labelled("x", "db-0", "db"),
        labelled("y", "cache-web", "cache"),
        labelled("y", "Api-Server", "api"),
    ]
}

/// The parts of `input`; a half-typed `!` is no filter (the bar keeps the last good one).
fn parts(input: &str) -> FilterParts {
    parse(input).map(|expr| expr.parts()).unwrap_or_default()
}

/// Applies `input` as the filter bar would (the sort follows a fuzzy filter).
fn type_filter(h: &mut Harness, sub: &mut Subscription, m: &mut Mirror, input: &str) {
    let parts = parts(input);
    let sort = parts.sort(None);
    sub.set_filter_parts(parts, sort);
    h.settle();
    m.drain(sub);
}

fn watches(h: &Harness) -> Vec<(Option<String>, Option<String>)> {
    h.resources
        .recorded_calls()
        .into_iter()
        .filter_map(|c| match c {
            ResourceCall::Watch {
                namespace,
                options: WatchOptions { label_selector, .. },
                ..
            } => Some((namespace, label_selector)),
            _ => None,
        })
        .collect()
}

#[test]
fn text_and_inverse_filter_by_name_without_restarting_the_feed() {
    let mut h = Harness::with_objects(fixtures());
    let mut sub = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    assert_eq!(m.rows.len(), 5);

    type_filter(&mut h, &mut sub, &mut m, "/WEB");
    assert_eq!(m.names(), ["x/web-1", "x/web-2", "y/cache-web"]);
    type_filter(&mut h, &mut sub, &mut m, "/!web");
    assert_eq!(m.names(), ["x/db-0", "y/Api-Server"]);
    type_filter(&mut h, &mut sub, &mut m, "^web-[12]$");
    assert_eq!(m.names(), ["x/web-1", "x/web-2"]);
    type_filter(&mut h, &mut sub, &mut m, "");
    assert_eq!(m.rows.len(), 5);
    assert_eq!(watches(&h).len(), 1, "client-side filters never re-key");
}

#[test]
fn the_total_counts_the_cache_not_the_filtered_rows() {
    let mut h = Harness::with_objects(fixtures());
    let mut sub = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    assert_eq!(m.last.as_ref().map(|d| (d.len, d.total)), Some((5, 5)));
    type_filter(&mut h, &mut sub, &mut m, "web");
    assert_eq!(m.last.as_ref().map(|d| (d.len, d.total)), Some((3, 5)));
}

#[test]
fn the_total_follows_adds_the_filter_hides() {
    let mut h = Harness::new();
    h.resources.script().watch.push_ok(timeline(vec![
        batch(vec![Delta::Restarted(fixtures())]),
        batch(vec![Delta::Applied(labelled("x", "db-9", "db"))]),
        batch(vec![Delta::Deleted(labelled("x", "db-0", "db"))]),
    ]));
    let mut sub = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    type_filter(&mut h, &mut sub, &mut m, "web");
    assert_eq!(m.last.as_ref().map(|d| (d.len, d.total)), Some((3, 5)));
    h.advance(1);
    m.drain(&mut sub);
    assert_eq!(m.last.as_ref().map(|d| (d.len, d.total)), Some((3, 6)));
    assert!(matches!(m.last_rows(), RowChange::Unchanged));
    h.advance(1);
    m.drain(&mut sub);
    assert_eq!(m.last.as_ref().map(|d| (d.len, d.total)), Some((3, 5)));
}

#[test]
fn a_label_selector_rekeys_the_feed_and_the_server_filters() {
    let mut h = Harness::with_options(options_with_grace(0));
    for r in fixtures() {
        h.resources.insert(r);
    }
    let mut sub = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    assert_eq!(m.rows.len(), 5);

    type_filter(&mut h, &mut sub, &mut m, "-l app=web");
    assert_eq!(m.names(), ["x/web-1", "x/web-2"]);
    assert_eq!(
        watches(&h),
        [(None, None), (None, Some("app=web".to_owned()))],
        "re-keyed with the selector, still cluster-wide"
    );
    assert_eq!(
        h.resources.live_watches(),
        1,
        "the unselected feed was released"
    );
    assert_eq!(
        sub.query()
            .selector
            .as_ref()
            .map(ToString::to_string)
            .as_deref(),
        Some("app=web")
    );
    assert_eq!(
        m.last.as_ref().map(|d| (d.len, d.total)),
        Some((2, 2)),
        "the total is the server's answer: the client never saw the rest"
    );

    // Another selector is another feed; the same one again in grace is reused.
    type_filter(&mut h, &mut sub, &mut m, "-l app!=web,app!=cache");
    assert_eq!(m.names(), ["x/db-0", "y/Api-Server"]);
    type_filter(&mut h, &mut sub, &mut m, "");
    assert_eq!(m.rows.len(), 5, "no selector: every object again");
    assert_eq!(watches(&h).len(), 4);
    assert_eq!(h.resources.live_watches(), 1);
}

#[test]
fn rekeying_keeps_the_old_rows_until_the_new_feed_lists() {
    let mut h = Harness::with_options(options_with_grace(0));
    for r in fixtures() {
        h.resources.insert(r);
    }
    let mut sub = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    // The selected feed lists after 5 s, with an object the client has never seen.
    h.resources.script().watch.push_ok(
        Timeline::new()
            .ok_at(
                Duration::from_secs(5),
                batch(vec![Delta::Restarted(vec![labelled("x", "new-1", "new")])]),
            )
            .keep_open(),
    );
    sub.set_selector(Some(LabelSelector::parse("app=new").unwrap()));
    h.settle();
    assert!(
        next(&mut sub).is_none(),
        "nothing is delivered (no empty flash) while the new feed has no data"
    );
    h.advance(5);
    m.drain(&mut sub);
    assert_eq!(m.names(), ["x/new-1"]);
    assert_eq!(sub.state(), FeedState::Ready);

    // A selector nothing matches: the previous rows stay until the server answers, then empty.
    h.resources.script().watch.push_ok(
        Timeline::new()
            .ok_at(
                Duration::from_secs(5),
                batch(vec![Delta::Restarted(vec![])]),
            )
            .keep_open(),
    );
    sub.set_selector(Some(LabelSelector::parse("app=none").unwrap()));
    h.settle();
    assert!(next(&mut sub).is_none());
    h.advance(5);
    m.drain(&mut sub);
    assert!(m.rows.is_empty());
    assert_eq!(sub.state(), FeedState::Ready);
}

/// Counts wakes, so a test can tell a parked consumer was woken (`now_or_never` cannot).
struct CountWake(std::sync::atomic::AtomicUsize);

impl futures::task::ArcWake for CountWake {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        arc_self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[test]
fn an_empty_first_list_wakes_the_consumer_parked_on_the_rekey_hold() {
    let mut h = Harness::with_options(options_with_grace(0));
    for r in fixtures() {
        h.resources.insert(r);
    }
    let mut sub = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    h.resources.script().watch.push_ok(
        Timeline::new()
            .ok_at(
                Duration::from_secs(5),
                batch(vec![Delta::Restarted(vec![])]),
            )
            .keep_open(),
    );
    sub.set_selector(Some(LabelSelector::parse("app=none").unwrap()));
    h.settle();
    // Park the way the view does: poll with a real waker and wait for it to fire.
    let wakes = Arc::new(CountWake(Default::default()));
    let waker = futures::task::waker(wakes.clone());
    let mut cx = std::task::Context::from_waker(&waker);
    assert!(sub.poll_next_unpin(&mut cx).is_pending());
    let before = wakes.0.load(std::sync::atomic::Ordering::SeqCst);
    h.advance(5);
    assert!(
        wakes.0.load(std::sync::atomic::Ordering::SeqCst) > before,
        "the empty list must wake the parked consumer"
    );
    m.drain(&mut sub);
    assert!(m.rows.is_empty());
}

#[test]
fn rekeying_shows_the_matching_cached_rows_at_once() {
    let mut h = Harness::with_options(options_with_grace(0));
    for r in fixtures() {
        h.resources.insert(r);
    }
    let mut sub = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    // The selected feed takes 5 s to list.
    h.resources.script().watch.push_ok(
        Timeline::new()
            .ok_at(
                Duration::from_secs(5),
                batch(vec![Delta::Restarted(vec![
                    labelled("x", "web-1", "web"),
                    labelled("x", "web-2", "web"),
                ])]),
            )
            .keep_open(),
    );
    sub.set_selector(Some(LabelSelector::parse("app=web").unwrap()));
    h.settle();
    m.drain(&mut sub);
    assert_eq!(
        m.names(),
        ["x/web-1", "x/web-2"],
        "seeded from the old feed's matches"
    );
    assert_eq!(sub.state(), FeedState::Warming);
    h.advance(5);
    m.drain(&mut sub);
    assert_eq!(m.names(), ["x/web-1", "x/web-2"]);
    assert_eq!(sub.state(), FeedState::Ready);
}

#[test]
fn a_selector_composes_with_the_namespace_scope_and_never_widens_it() {
    let mut h = Harness::with_options(options_with_grace(0));
    for r in fixtures() {
        h.resources.insert(r);
    }
    let mut sub = h.subscribe(in_namespaces(pods(), &["x"]));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    type_filter(&mut h, &mut sub, &mut m, "-l app=web");
    assert_eq!(m.names(), ["x/web-1", "x/web-2"]);
    assert!(
        watches(&h).iter().all(|(ns, _)| ns.as_deref() == Some("x")),
        "{:?}",
        watches(&h)
    );

    // The namespace selection changes with the selector active: still selected, still scoped.
    sub.rescope(WatchScope::Namespaces(vec!["y".into()]));
    h.settle();
    m.drain(&mut sub);
    assert!(m.rows.is_empty(), "y has no app=web pod: {:?}", m.names());
    assert_eq!(
        watches(&h).last(),
        Some(&(Some("y".to_owned()), Some("app=web".to_owned())))
    );
    assert!(
        watches(&h).iter().all(|(ns, _)| ns.is_some()),
        "never cluster-wide"
    );
    assert_eq!(
        sub.query().scope,
        WatchScope::Namespaces(vec!["y".into()]),
        "the filter does not touch the scope"
    );

    // Clearing the selector widens nothing either.
    type_filter(&mut h, &mut sub, &mut m, "");
    assert_eq!(m.names(), ["y/Api-Server", "y/cache-web"]);
    assert!(watches(&h).iter().all(|(ns, _)| ns.is_some()));
}

#[test]
fn a_selector_and_a_client_filter_each_apply_where_they_belong() {
    let mut h = Harness::with_objects(fixtures());
    let mut sub = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    type_filter(&mut h, &mut sub, &mut m, "-l app=web");
    type_filter(&mut h, &mut sub, &mut m, "web-2");
    assert_eq!(
        m.names(),
        ["x/web-2"],
        "switching to text drops the selector"
    );
    assert_eq!(sub.query().selector, None);
    assert_eq!(m.last.as_ref().map(|d| d.total), Some(5));
}

#[test]
fn fuzzy_filters_rank_best_match_first_and_ties_are_stable() {
    let mut h = Harness::with_objects([
        labelled("x", "a-x-b-x-c", "t"),
        labelled("x", "abc-pod", "t"),
        labelled("x", "xaxbxc", "t"),
        labelled("x", "a-b-c", "t"),
        labelled("x", "no-match", "t"),
    ]);
    let mut sub = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    type_filter(&mut h, &mut sub, &mut m, "-f abc");
    assert_eq!(
        m.names(),
        ["x/abc-pod", "x/a-b-c", "x/a-x-b-x-c", "x/xaxbxc"]
    );
    assert_eq!(sub.query().sort.field, SortField::Relevance);
    // Typing more re-ranks the survivors (a narrowing pass).
    type_filter(&mut h, &mut sub, &mut m, "-f abcp");
    assert_eq!(m.names(), ["x/abc-pod"]);
    type_filter(&mut h, &mut sub, &mut m, "-f ab");
    assert_eq!(
        m.names(),
        ["x/abc-pod", "x/a-b-c", "x/a-x-b-x-c", "x/xaxbxc"],
        "deleting recomputes from the cache"
    );
    // A column sort wins over the ranking, and stays a plain filter.
    let parts = parts("-f ab");
    sub.set_filter_parts(parts, SortKey::by(SortField::Name));
    h.settle();
    m.drain(&mut sub);
    assert_eq!(
        m.names(),
        ["x/a-b-c", "x/a-x-b-x-c", "x/abc-pod", "x/xaxbxc"]
    );
}

#[test]
fn objects_arriving_while_filtered_are_ranked_and_filtered_too() {
    let mut h = Harness::new();
    h.resources.script().watch.push_ok(timeline(vec![
        batch(vec![Delta::Restarted(fixtures())]),
        batch(vec![
            Delta::Applied(labelled("x", "web-9", "web")),
            Delta::Applied(labelled("x", "db-9", "db")),
            Delta::Deleted(labelled("x", "web-1", "web")),
        ]),
    ]));
    let mut sub = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    type_filter(&mut h, &mut sub, &mut m, "web");
    h.advance(1);
    m.drain(&mut sub);
    assert_eq!(m.names(), ["x/web-2", "x/web-9", "y/cache-web"]);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// Typing and deleting characters through a live subscription (narrowing in place when it
    /// can, re-seeding from the cache when it cannot) always shows the rows a fresh subscription
    /// with that filter shows.
    #[test]
    fn typing_and_deleting_matches_a_fresh_subscription(
        names in proptest::collection::vec("[a-c-]{1,6}", 1..30),
        edits in proptest::collection::vec((any::<bool>(), "[a-c]"), 1..10),
        mode in 0u8..4,
    ) {
        let prefix = ["", "!", "-f ", "!-f "][usize::from(mode)];
        let objects: Vec<Resource> = names
            .iter()
            .enumerate()
            .map(|(i, n)| labelled("x", &format!("{n}{i}"), "t"))
            .collect();
        let mut h = Harness::with_objects(objects.clone());
        let mut sub = h.subscribe(all(pods()));
        let mut m = Mirror::default();
        m.drain(&mut sub);
        let mut text = String::new();
        for (add, ch) in edits {
            if add { text.push_str(&ch) } else { text.pop(); }
            let input = format!("{prefix}{text}");
            type_filter(&mut h, &mut sub, &mut m, &input);

            let mut fresh_h = Harness::with_objects(objects.clone());
            let parts = parts(&input);
            let sort = parts.sort(None);
            let mut fresh = fresh_h.subscribe(all(pods()).with_filter(parts.filter).with_sort(sort));
            let mut fm = Mirror::default();
            fm.drain(&mut fresh);
            prop_assert_eq!(m.names(), fm.names(), "after editing to {:?}", input);
        }
    }
}
