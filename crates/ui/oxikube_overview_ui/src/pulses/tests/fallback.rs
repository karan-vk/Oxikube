//! Kinds the store cannot count: the watch budget refuses their feed, and the page asks the API
//! server for the count once instead, marked as such, and again on `r`.

use std::sync::Arc;

use gpui::TestAppContext;
use oxikube_app::CountState;
use oxikube_app::store::{MaxFeeds, StoreOptions};
use oxikube_domain::OxiError;
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::ListPage;
use oxikube_testkit::{ResourceCall, pod};

use super::{Fixture, cluster, no_grace};
use crate::pulses::{FALLBACK_INTERVAL, Level, SETTLE_TICKS};

fn refuse_all() -> StoreOptions {
    StoreOptions {
        budget: Arc::new(MaxFeeds::new(0)),
        ..no_grace()
    }
}

fn meta_page(held: usize, remaining: Option<i64>) -> ListPage<oxikube_domain::ObjectMeta> {
    ListPage {
        items: (0..held)
            .map(|i| oxikube_domain::ObjectMeta::named(format!("o{i}")))
            .collect(),
        continue_token: remaining.map(|_| "next".to_owned()),
        resource_version: None,
        remaining_item_count: remaining,
    }
}

fn metadata_calls(fx: &Fixture) -> usize {
    fx.ports
        .resources
        .recorded_calls()
        .iter()
        .filter(|call| matches!(call, ResourceCall::ListMetadata { .. }))
        .count()
}

#[gpui::test]
fn a_kind_over_the_budget_is_counted_once_from_the_api_server(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, refuse_all(), |ports| {
        ports
            .resources
            .insert(pod().namespace("a").name("p1").running().build());
        // Every one of the 14 kinds is asked once; pods have 4 pods.
        for _ in 0..14 {
            ports
                .resources
                .script()
                .list_metadata
                .push_ok(meta_page(1, Some(3)));
        }
    });
    fx.ticks(2);
    assert!(
        matches!(fx.count_of("pods"), Some(CountState::OverBudget { .. })),
        "{:?}",
        fx.count_of("pods")
    );
    assert_eq!(
        metadata_calls(&fx),
        14,
        "a refused kind is asked for once, at once"
    );

    let pods = fx.read(|p| p.model().tile("pods").expect("pods").face().clone());
    assert_eq!(pods.value, "4");
    assert!(pods.caption.starts_with("Counted once"), "{}", pods.caption);
    assert_eq!(pods.level, Level::Muted);
    assert!(pods.hover.is_some(), "the budget's reason is on hover");
    assert!(
        fx.read(|p| p.model().tile("pods").expect("pods").ring.len()) >= 1,
        "and sampled"
    );
}

#[gpui::test]
fn the_count_is_asked_again_only_when_it_is_due(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, refuse_all(), |_| {});
    fx.ticks(SETTLE_TICKS as usize);
    let first = metadata_calls(&fx);
    assert_eq!(first, 14);
    fx.ticks(10);
    assert_eq!(metadata_calls(&fx), first, "not again before the interval");
    fx.ticks(FALLBACK_INTERVAL.as_secs() as usize);
    assert_eq!(metadata_calls(&fx), first * 2, "again after it");
}

#[gpui::test]
fn r_counts_again_now(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, refuse_all(), |_| {});
    fx.ticks(2);
    let first = metadata_calls(&fx);
    fx.keys("r");
    assert_eq!(
        metadata_calls(&fx),
        first + 14,
        "every unwatched kind is counted again"
    );
}

#[gpui::test]
fn a_forbidden_count_is_an_error_not_zero(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, refuse_all(), |ports| {
        for _ in 0..14 {
            ports
                .resources
                .script()
                .list_metadata
                .push_err(OxiError::forbidden("pods is forbidden"));
        }
    });
    fx.ticks(2);
    let tile = fx.read(|p| p.model().tile("pods").expect("pods").face().clone());
    assert_eq!(tile.value, "–");
    assert_eq!(tile.hover.as_deref(), Some("pods is forbidden"));
}

#[gpui::test]
fn a_kind_with_a_feed_is_never_counted_by_request(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), |ports| {
        ports
            .resources
            .insert(pod().namespace("a").name("p1").running().build());
    });
    fx.ticks(10);
    assert_eq!(
        metadata_calls(&fx),
        0,
        "the feeds count; nothing asks the API server"
    );
}

#[gpui::test]
fn a_new_namespace_selection_counts_the_unwatched_kinds_again(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, refuse_all(), |ports| {
        // Under every namespace pods are 4; under namespace "b" they are 2.
        for _ in 0..14 {
            ports
                .resources
                .script()
                .list_metadata
                .push_ok(meta_page(1, Some(3)));
        }
        for _ in 0..14 {
            ports
                .resources
                .script()
                .list_metadata
                .push_ok(meta_page(1, Some(1)));
        }
    });
    fx.ticks(2);
    let value = |fx: &mut Fixture, id: &'static str| {
        fx.read(move |p| p.model().tile(id).expect("a tile").face().value.clone())
    };
    assert_eq!(value(&mut fx, "pods"), "4");
    let first = metadata_calls(&fx);

    fx.sessions
        .set_namespace_selection(&cluster(), NamespaceSelection::single("b"))
        .expect("selected");
    fx.vcx.run_until_parked();
    assert_eq!(
        value(&mut fx, "pods"),
        "2",
        "the count of the old selection is not shown under the new one, and the new one is asked \
         for at once, not after the interval"
    );
    // The cluster-scoped kinds do not depend on the selection and are not asked again.
    let namespaced = fx.read(|p| {
        p.model()
            .tiles()
            .iter()
            .filter(|t| {
                t.spec
                    .target()
                    .is_some_and(|t| t.scope == oxikube_domain::ids::Scope::Namespaced)
            })
            .count()
    });
    assert_eq!(metadata_calls(&fx), first + namespaced);
}

#[gpui::test]
fn a_selection_change_drops_the_counts_in_flight(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, refuse_all(), |_| {});
    fx.ticks(2);
    // A request is out for a namespaced kind, as if it had not answered yet.
    fx.vcx.update(|_, cx| {
        fx.page.update(cx, |page, _| {
            let pods = page
                .model
                .tiles()
                .iter()
                .position(|t| t.spec.id == "pods")
                .expect("pods");
            page.recounts[pods].mark_out_for_test();
        });
    });
    fx.sessions
        .set_namespace_selection(&cluster(), NamespaceSelection::single("b"))
        .expect("selected");
    fx.vcx.run_until_parked();
    assert!(
        !fx.read(|p| p.recounts.iter().any(|r| r.is_out())),
        "nothing from the old selection is still out"
    );
}
