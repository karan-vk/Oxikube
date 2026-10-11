//! The recent-warnings panel: the empty state, the rows the hook gives, the cap, and the
//! disconnected cluster (no warnings, no panel).

use std::sync::Arc;

use gpui::TestAppContext;
use oxikube_app::{FixedWarnings, WarningRow};

use super::Fixture;
use crate::overview_lite::{WARNING_ROWS, headline};

fn warning(object: &str, count: u32) -> WarningRow {
    WarningRow {
        namespace: Some("shop".into()),
        object: object.to_owned(),
        reason: "BackOff".to_owned(),
        message: "Back-off restarting failed container".to_owned(),
        count,
        last_seen: None,
    }
}

fn draw(fx: &mut Fixture) {
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
}

#[test]
fn a_headline_names_the_object_the_reason_and_a_repeat_count() {
    assert_eq!(
        headline(&warning("Pod/web-0", 1)),
        "shop/Pod/web-0 · BackOff"
    );
    assert_eq!(
        headline(&warning("Pod/web-0", 7)),
        "shop/Pod/web-0 · BackOff ×7"
    );
    let node = WarningRow {
        namespace: None,
        ..warning("Node/n1", 1)
    };
    assert_eq!(headline(&node), "Node/n1 · BackOff");
}

#[gpui::test]
fn with_no_events_feed_the_panel_says_there_are_no_warnings(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, super::no_grace());
    fx.tick();
    draw(&mut fx);
    assert!(fx.vcx.debug_bounds("overview-warnings").is_some());
    assert!(fx.vcx.debug_bounds("overview-warnings-empty").is_some());
    assert!(fx.vcx.debug_bounds("overview-warning-0").is_none());
}

#[gpui::test]
fn the_panel_lists_the_newest_warnings_up_to_its_cap(cx: &mut TestAppContext) {
    let many: Vec<WarningRow> = (0..WARNING_ROWS + 4)
        .map(|i| warning(&format!("Pod/p{i}"), 1))
        .collect();
    let mut fx = Fixture::open_with_warnings(cx, Arc::new(FixedWarnings(many)));
    fx.tick();
    draw(&mut fx);
    let shown = fx.vcx.update(|_, cx| fx.overview.read(cx).warnings().len());
    assert_eq!(shown, WARNING_ROWS);
    assert!(fx.vcx.debug_bounds("overview-warning-0").is_some());
    assert!(fx.vcx.debug_bounds("overview-warnings-empty").is_none());
    assert_eq!(WARNING_ROWS, 8, "the selectors below name the cap");
    assert!(fx.vcx.debug_bounds("overview-warning-7").is_some());
    assert!(
        fx.vcx.debug_bounds("overview-warning-8").is_none(),
        "no more than the cap"
    );
}

#[gpui::test]
fn a_disconnected_cluster_has_no_warnings(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_disconnected(cx);
    draw(&mut fx);
    assert!(fx.vcx.debug_bounds("overview-warnings").is_none());
    let shown = fx.vcx.update(|_, cx| fx.overview.read(cx).warnings().len());
    assert_eq!(shown, 0);
}
