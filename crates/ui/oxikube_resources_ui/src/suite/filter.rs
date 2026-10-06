//! The filter bar end to end over a live feed: `/text`, `/!text`, `/-l selector`, `/-f fuzzy`,
//! the `shown of total` count, and a filter that keeps applying as the feed changes.

use gpui::TestAppContext;
use oxikube_domain::Resource;
use oxikube_testkit::{ScriptedFeed, TICK, pod};

use super::Scripted;
use crate::filter::SELECTOR_DEBOUNCE;

fn labelled(ns: &str, name: &str, app: &str) -> Resource {
    pod().namespace(ns).name(name).label("app", app).build()
}

fn fixtures() -> Vec<Resource> {
    vec![
        labelled("x", "web-1", "web"),
        labelled("x", "web-2", "web"),
        labelled("x", "db-0", "db"),
        labelled("y", "cache-web", "cache"),
        labelled("y", "api", "api"),
    ]
}

/// Types `keys` into the filter bar (`/` focuses it), then lets the debounce and the store settle.
fn type_filter(s: &mut Scripted, keys: &str) {
    s.f.keys(&s.table, "/");
    s.f.vcx.simulate_keystrokes(keys);
    s.f.vcx.executor().advance_clock(SELECTOR_DEBOUNCE * 2);
    s.f.settle();
}

fn count(s: &mut Scripted) -> Option<String> {
    let bar = s.f.vcx.update(|_, cx| s.table.read(cx).filter().clone());
    s.f.vcx.update(|_, cx| bar.read(cx).count_label())
}

#[gpui::test]
fn text_and_negated_filters_keep_applying_as_the_feed_changes(cx: &mut TestAppContext) {
    let feed = ScriptedFeed::new()
        .initial(fixtures())
        .add(1, labelled("x", "web-3", "web"))
        .add(1, labelled("x", "db-1", "db"))
        .delete(2, labelled("x", "web-1", "web"))
        .add(3, labelled("x", "web-9", "web"))
        .delete(3, labelled("y", "api", "api"));
    let mut s = Scripted::open(cx, &feed);
    assert_eq!(s.names().len(), 5);
    assert_eq!(count(&mut s), None, "no filter, no count");

    // `/web`: a substring match on the row.
    type_filter(&mut s, "w e b");
    assert_eq!(s.names(), ["web-1", "web-2", "cache-web"]);
    assert_eq!(count(&mut s).as_deref(), Some("3 of 5"));

    s.step();
    assert_eq!(
        s.names(),
        ["web-1", "web-2", "web-3", "cache-web"],
        "a pod that matches appears, one that does not stays out"
    );
    assert_eq!(count(&mut s).as_deref(), Some("4 of 7"));
    s.step();
    assert_eq!(
        s.names(),
        ["web-2", "web-3", "cache-web"],
        "a deleted match goes"
    );
    assert_eq!(count(&mut s).as_deref(), Some("3 of 6"));

    // `/!web`: the inverse, over the rows the feed holds now.
    s.f.vcx.simulate_keystrokes("escape");
    s.f.settle();
    type_filter(&mut s, "! w e b");
    assert_eq!(s.names(), ["db-0", "db-1", "api"]);
    assert_eq!(count(&mut s).as_deref(), Some("3 of 6"));
    s.step();
    assert_eq!(
        s.names(),
        ["db-0", "db-1"],
        "a new `web` pod stays out of the inverse, a deleted match leaves it"
    );
    assert_eq!(count(&mut s).as_deref(), Some("2 of 6"));

    // Escape clears it.
    s.f.vcx.simulate_keystrokes("escape");
    s.f.settle();
    assert_eq!(s.names().len(), 6);
    assert_eq!(count(&mut s), None);
}

#[gpui::test]
fn a_label_selector_filters_on_the_server_and_follows_its_feed(cx: &mut TestAppContext) {
    let first = ScriptedFeed::new().initial(fixtures());
    let mut s = Scripted::open(cx, &first);
    // The selector re-keys the feed: the next watch is the server's answer for `app=web`.
    let selected = ScriptedFeed::new()
        .initial([labelled("x", "web-1", "web"), labelled("x", "web-2", "web")])
        .add(1, labelled("x", "web-3", "web"));
    selected.install(&s.f.ports().resources);

    type_filter(&mut s, "- l space a p p = w e b enter");
    assert_eq!(s.names(), ["web-1", "web-2"]);
    assert_eq!(count(&mut s).as_deref(), Some("2 of 2"));

    s.f.ports().resources.clock().advance(TICK);
    s.f.settle();
    assert_eq!(
        s.names(),
        ["web-1", "web-2", "web-3"],
        "the re-keyed feed keeps delivering"
    );
}

#[gpui::test]
fn a_fuzzy_filter_ranks_best_match_first_as_the_feed_changes(cx: &mut TestAppContext) {
    let feed = ScriptedFeed::new()
        .initial([
            pod().namespace("x").name("a-x-b-x-c").build(),
            pod().namespace("x").name("abc-pod").build(),
            pod().namespace("x").name("other").build(),
        ])
        .add(1, pod().namespace("x").name("a-b-c").build())
        .add(1, pod().namespace("x").name("unrelated").build())
        .add(2, pod().namespace("x").name("abc").build());
    let mut s = Scripted::open(cx, &feed);
    type_filter(&mut s, "- f space a b c enter");
    assert_eq!(s.names(), ["abc-pod", "a-x-b-x-c"]);

    s.step();
    assert_eq!(
        s.names(),
        ["abc-pod", "a-b-c", "a-x-b-x-c"],
        "a new match is ranked among the others, a non-match stays out"
    );
    s.step();
    assert_eq!(s.names()[0], "abc", "the exact match ranks first");
    assert_eq!(s.names().len(), 4);
    assert_eq!(count(&mut s).as_deref(), Some("4 of 6"));
}
