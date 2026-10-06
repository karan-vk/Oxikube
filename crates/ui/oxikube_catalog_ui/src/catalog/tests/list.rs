//! What the list shows: rows, their facts, the loading and empty states, ordering.

use gpui::TestAppContext;
use jiff::Timestamp;
use oxikube_ports::{StateKey, StatePort as _, StateTable};
use serde_json::json;

use super::{Fixture, Setup, contexts, id, named};
use crate::catalog::LoadState;
use crate::catalog::test_support::context;
use crate::catalog::{EMPTY_STEPS, LOADING_TEXT};

#[gpui::test]
fn the_first_frame_is_the_loading_state_and_the_rows_follow(cx: &mut TestAppContext) {
    let mut f = Fixture::start(cx, Setup::new(contexts(3)).gated());
    // The window is up and has settled while the read is still waiting: that is the first
    // frame of the app, before any file has been read.
    assert_eq!(
        f.read(|v| v.model().load_state().clone()),
        LoadState::Loading
    );
    assert!(f.read(|v| v.model().total()) == 0);
    f.window.draw_frame();
    assert!(
        f.is_laid_out("catalog-loading"),
        "the loading state is on screen"
    );
    assert!(
        !f.is_laid_out("catalog-empty"),
        "loading is not 'no clusters'"
    );

    f.open_gate();
    assert_eq!(f.read(|v| v.model().load_state().clone()), LoadState::Ready);
    f.window.draw_frame();
    assert!(!f.is_laid_out("catalog-loading"));
    assert!(f.is_laid_out("catalog-row-0"));
    assert!(LOADING_TEXT.starts_with("Reading"));
}

#[gpui::test]
fn every_context_is_listed_with_its_cluster_user_and_source_file(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, contexts(20));
    f.window.draw_frame();
    assert_eq!(f.names().len(), 20);
    for ix in 0..20 {
        assert!(
            f.is_laid_out(format!("catalog-row-{ix}")),
            "row {ix} was not rendered"
        );
        assert!(f.is_laid_out(format!("catalog-status-{ix}")), "badge {ix}");
        assert!(f.is_laid_out(format!("catalog-star-{ix}")), "star {ix}");
    }
    assert!(!f.is_laid_out("catalog-row-20"), "no phantom row");
    assert!(!f.is_laid_out("catalog-empty"));

    // What each row is built from.
    let rows: Vec<_> = f.read(|v| {
        v.model()
            .visible_rows()
            .map(|r| {
                (
                    r.name().to_string(),
                    r.cluster().to_string(),
                    r.user().to_string(),
                    r.source().to_string(),
                )
            })
            .collect()
    });
    assert_eq!(
        rows[3],
        (
            "ctx-03".to_owned(),
            "ctx-03-cluster".to_owned(),
            "ctx-03-user".to_owned(),
            "~/.kube/config".to_owned()
        )
    );
}

#[gpui::test]
fn the_count_in_the_header_follows_the_catalog(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, contexts(3));
    f.window.draw_frame();
    assert!(f.is_laid_out("catalog-count:3 clusters"));

    f.type_text("ctx-01");
    f.window.draw_frame();
    assert!(
        f.is_laid_out("catalog-count:1 of 3"),
        "the search narrows it"
    );

    f.keys("escape");
    f.source.set_contexts(contexts(1));
    f.app.run_until_parked();
    f.window.draw_frame();
    assert!(f.is_laid_out("catalog-count:1 cluster"));
}

#[gpui::test]
fn an_empty_catalog_explains_how_to_add_kubeconfigs(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, vec![]);
    f.window.draw_frame();
    assert!(
        f.is_laid_out("catalog-empty"),
        "the empty state is on screen"
    );
    assert!(!f.is_laid_out("catalog-row-0"));
    assert!(!f.is_laid_out("catalog-loading"));
    for ix in 0..EMPTY_STEPS.len() {
        assert!(
            f.is_laid_out(format!("catalog-empty-step-{ix}")),
            "step {ix} is rendered"
        );
    }
    let steps = EMPTY_STEPS.join("\n");
    assert!(steps.contains("~/.kube/config"), "names the default path");
    assert!(
        steps.contains("KUBECONFIG"),
        "names the environment variable"
    );
    assert!(
        steps.contains("Kubeconfig sources"),
        "points at sources management"
    );
    assert!(steps.contains("paste"), "mentions pasting");
}

#[gpui::test]
fn a_catalog_that_fills_later_replaces_the_empty_state(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, vec![]);
    f.window.draw_frame();
    assert!(f.is_laid_out("catalog-empty"));
    f.source.set_contexts(contexts(2));
    f.app.run_until_parked();
    f.window.draw_frame();
    assert!(!f.is_laid_out("catalog-empty"));
    assert!(f.is_laid_out("catalog-row-1"));
}

#[gpui::test]
fn a_source_that_cannot_be_read_says_so_instead_of_claiming_there_are_no_clusters(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::start(
        cx,
        Setup::new(contexts(2)).prepare(|parts| {
            parts
                .source
                .script()
                .contexts
                .push_err(oxikube_domain::OxiError::internal(
                    "cannot read ~/.kube/config",
                ));
        }),
    );
    f.window.draw_frame();
    assert!(f.is_laid_out("catalog-failed"));
    assert!(!f.is_laid_out("catalog-empty"));
    assert!(matches!(
        f.read(|v| v.model().load_state().clone()),
        LoadState::Failed(m) if m.contains("cannot read")
    ));
}

#[gpui::test]
fn favourites_are_listed_first_then_the_most_recently_used(cx: &mut TestAppContext) {
    let mut f = Fixture::start(
        cx,
        Setup::new(named(&["alpha", "bravo", "charlie", "delta"])).prepare(|parts| {
            let table = StateTable::new("cluster_catalog").unwrap();
            let put = |name: &str, row: serde_json::Value| {
                let key = StateKey::new(id(name).as_str()).unwrap();
                futures::executor::block_on(parts.state.table_put(&table, &key, row)).unwrap();
            };
            put("delta", json!({ "favourite": true }));
            put(
                "charlie",
                json!({ "last_used": Timestamp::from_second(2_000).unwrap() }),
            );
            put(
                "bravo",
                json!({ "last_used": Timestamp::from_second(1_000).unwrap() }),
            );
        }),
    );
    assert_eq!(f.names(), ["delta", "charlie", "bravo", "alpha"]);
}

#[test]
fn last_used_is_shown_relative_to_now_and_never_for_a_new_cluster() {
    use crate::catalog::view::last_used_text;
    let now = Timestamp::from_second(10_000).unwrap();
    assert_eq!(last_used_text(None, now), "never");
    assert_eq!(last_used_text(Some(now), now), "just now");
    assert_eq!(
        last_used_text(Some(Timestamp::from_second(10_000 - 300).unwrap()), now),
        "5m ago"
    );
    assert_eq!(
        last_used_text(Some(Timestamp::from_second(10_000 - 7_300).unwrap()), now),
        "2h ago"
    );
    assert_eq!(
        last_used_text(
            Some(Timestamp::from_second(10_000 - 3 * 86_400).unwrap()),
            now
        ),
        "3d ago"
    );
    assert_eq!(
        last_used_text(
            Some(Timestamp::from_second(10_000 - 65 * 86_400).unwrap()),
            now
        ),
        "2mo ago"
    );
    assert_eq!(
        last_used_text(
            Some(Timestamp::from_second(10_000 - 800 * 86_400).unwrap()),
            now
        ),
        "2y ago"
    );
    assert_eq!(
        last_used_text(Some(Timestamp::from_second(20_000).unwrap()), now),
        "just now",
        "a clock in the past never prints a negative age"
    );
}

#[gpui::test]
fn a_context_that_cannot_work_is_kept_with_its_reason(cx: &mut TestAppContext) {
    let mut broken = context("broken");
    broken.problem = Some("cluster \"gone\" is not defined in the kubeconfig".into());
    let mut f = Fixture::open(cx, vec![context("fine"), broken]);
    f.window.draw_frame();
    assert_eq!(
        f.names().len(),
        2,
        "an invalid entry does not hide, and does not hide the rest"
    );
    let badges: Vec<_> = (0..2)
        .map(|ix| f.read(|v| v.model().badge(ix)).unwrap())
        .collect();
    let invalid = badges
        .iter()
        .find(|b| b.label == "Invalid")
        .expect("an Invalid badge");
    assert_eq!(invalid.tone, crate::catalog::Tone::Error);
    assert!(
        invalid
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("gone"))
    );
    assert!(f.is_laid_out("catalog-row-1"));
}
