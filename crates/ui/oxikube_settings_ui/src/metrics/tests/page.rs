//! What the page draws for each state, in a workspace window.

use gpui::TestAppContext;

use super::Harness;
use crate::metrics::test_support::{
    detecting, disabled, forbidden, found, manual, not_found, probe_failed,
};
use crate::metrics::{ITEM_KEY, Tone};
use oxikube_workspace::Item as _;

#[gpui::test]
fn a_found_provider_shows_its_headline_and_the_services_looked_at(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, found());
    assert!(h.is_laid_out("metrics-settings"));
    assert!(h.is_laid_out("metrics-headline:good"));
    assert!(!h.is_laid_out("metrics-unavailable"));
    for ix in 0..3 {
        assert!(h.is_laid_out(&format!(
            "metrics-probe-{ix}:{}",
            ["muted", "good", "muted"][ix]
        )));
    }
    assert!(h.is_laid_out("metrics-setting-Provider"));
    assert!(h.is_laid_out("metrics-edit-hint"));
    assert!(h.is_laid_out("metrics-cluster"));
}

#[gpui::test]
fn nothing_found_says_metrics_unavailable_and_still_lists_what_was_probed(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, not_found());
    assert!(h.is_laid_out("metrics-headline:warning"));
    assert!(h.is_laid_out("metrics-unavailable"));
    for ix in 0..3 {
        assert!(
            h.is_laid_out(&format!("metrics-probe-{ix}:muted")),
            "probe {ix}"
        );
    }
    h.read(|view| {
        assert!(
            view.model()
                .unavailable
                .as_deref()
                .unwrap()
                .starts_with("metrics unavailable: ")
        );
    });
}

#[gpui::test]
fn forbidden_is_shown_as_an_error_with_the_way_out(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, forbidden());
    assert!(h.is_laid_out("metrics-headline:error"));
    assert!(h.is_laid_out("metrics-unavailable"));
    assert!(h.is_laid_out("metrics-probe-0:warning"));
    h.read(|view| {
        assert!(
            view.model()
                .unavailable
                .as_deref()
                .unwrap()
                .contains("configure the endpoint by hand")
        );
    });
}

#[gpui::test]
fn a_service_that_did_not_answer_is_an_error_row(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, probe_failed());
    assert!(h.is_laid_out("metrics-headline:error"));
    assert!(h.is_laid_out("metrics-probe-0:error"));
}

#[gpui::test]
fn a_manual_endpoint_lists_its_settings(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, manual());
    for label in [
        "Enabled",
        "Provider",
        "Stack",
        "URL",
        "Path prefix",
        "Authentication",
    ] {
        assert!(
            h.is_laid_out(&format!("metrics-setting-{label}")),
            "{label}"
        );
    }
}

#[gpui::test]
fn the_page_follows_the_status_without_being_reopened(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, detecting());
    assert!(h.is_laid_out("metrics-headline:pending"));
    h.set(found());
    assert!(h.is_laid_out("metrics-headline:good"));
    assert!(!h.is_laid_out("metrics-headline:pending"));
    h.set(not_found());
    assert!(h.is_laid_out("metrics-headline:warning"));
    h.set(disabled());
    assert!(h.is_laid_out("metrics-headline:muted"));
    h.read(|view| assert_eq!(view.model().headline.tone, Tone::Muted));
}

#[gpui::test]
fn the_page_is_one_tab_per_cluster_tab(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, found());
    h.vcx.update(|_, cx| {
        let view = h.view.read(cx);
        assert_eq!(view.item_key(cx).as_deref(), Some(ITEM_KEY));
        let tab = view.tab_content(cx);
        assert_eq!(tab.title.as_ref(), "Metrics");
    });
}
