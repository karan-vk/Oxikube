//! A failed stream is shown once: one human sentence in the recovery strip, the raw text behind
//! its Details toggle, no red row in the body, and Close instead of a futile Reconnect when the
//! pod is gone.

use gpui::TestAppContext;
use oxikube_app::logs::LogState;
use oxikube_domain::OxiError;
use oxikube_testkit::Timeline;

use super::fixture::{Fx, lines};
use crate::view::{OpenLogs, Recovery};

fn open_failing(fx: &mut Fx, error: OxiError) -> gpui::Entity<crate::view::LogView> {
    fx.ports.logs.script().stream_logs.push_err(error);
    let views = fx.views.clone();
    let view = fx
        .vcx
        .update(|window, cx| {
            views.update(cx, |views, cx| {
                views.open(&super::fixture::pod_ref(), &OpenLogs::default(), window, cx)
            })
        })
        .unwrap();
    fx.settle();
    view
}

#[gpui::test]
fn a_failure_is_one_sentence_with_its_raw_text_behind_details(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open_failing(
        &mut fx,
        OxiError::forbidden("pods \"web-0\" is forbidden: cannot get pods/log"),
    );
    fx.read(&view, |view| {
        assert!(matches!(view.line_window().state(), LogState::Failed(_)));
        assert_eq!(view.recovery(), Some(Recovery::Reconnect));
        let error = view.recovery_error().expect("a failure");
        assert_eq!(error.summary(), "You do not have permission to do that.");
        assert!(error.raw().starts_with("pods \"web-0\" is forbidden"));
        // Said once: the strip has it, the body has no row for it.
        assert_eq!(view.line_window().row_count(), 0, "no red state row");
        assert!(!view.error_details_open());
    });
    fx.draw();
    assert!(fx.drawn("log-recovery"));
    assert!(fx.drawn("log-recovery-summary"));
    assert!(fx.drawn("log-reconnect"));
    assert!(!fx.drawn("log-close"));
    assert!(fx.drawn("log-details-toggle"));
    assert!(!fx.drawn("log-details"), "the raw text starts collapsed");

    fx.click("log-details-toggle");
    assert!(fx.drawn("log-details"), "Details opens the raw text");
    fx.click("log-details-toggle");
    assert!(!fx.drawn("log-details"), "and closes it");
}

#[gpui::test]
fn no_internal_error_label_reaches_the_strip(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open_failing(
        &mut fx,
        OxiError::internal("dial tcp 10.0.0.12:6443: connect: connection refused"),
    );
    let error = fx.read(&view, |v| v.recovery_error()).unwrap();
    assert_eq!(
        error.summary(),
        "The cluster's API server refused the connection."
    );
    assert!(!error.summary().contains("internal error"));
    assert!(!error.raw().contains("internal error"));
}

#[gpui::test]
fn a_pod_that_is_gone_offers_close_not_reconnect(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open_failing(&mut fx, OxiError::not_found("pods \"web-0\" not found"));
    fx.read(&view, |view| {
        assert_eq!(view.recovery(), Some(Recovery::Close));
        assert_eq!(
            view.recovery_error().unwrap().summary(),
            "This pod was not found. It may have been deleted."
        );
    });
    fx.draw();
    assert!(fx.drawn("log-close"));
    assert!(!fx.drawn("log-reconnect"), "reconnecting cannot succeed");

    // The `r` key does not reopen a read that cannot succeed.
    let opened = fx.opened().len();
    fx.keys("r");
    assert_eq!(fx.opened().len(), opened);

    // Close closes the tab.
    assert_eq!(fx.read(&view, |v| v.target().name.to_string()), "web-0");
    fx.click("log-close");
    let open = fx.vcx.update(|_, cx| {
        fx.workspace
            .read(cx)
            .items_of_type::<crate::view::LogView>()
            .len()
    });
    assert_eq!(open, 0, "the log tab is closed");
}

#[gpui::test]
fn a_merged_view_of_a_missing_deployment_offers_close(cx: &mut TestAppContext) {
    let mut fx = Fx::merged(cx);
    // No deployment in the cluster.
    let view = fx.open_web();
    fx.read(&view, |view| {
        assert_eq!(view.recovery(), Some(Recovery::Close));
        assert_eq!(view.line_window().row_count(), 0);
        let error = view.recovery_error().unwrap();
        assert_eq!(
            error.summary(),
            "This deployment was not found. It may have been deleted."
        );
    });
    fx.draw();
    assert!(fx.drawn("log-close"));
}

#[gpui::test]
fn a_stream_that_is_reconnecting_says_it_in_words_not_in_raw_text(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    // A followed stream that closes while the pod is still there is retried by the driver.
    let view = fx.open(Timeline::immediate(lines(0, 2)));
    fx.read(&view, |view| {
        assert!(matches!(
            view.line_window().state(),
            LogState::Reconnecting { .. }
        ));
        assert_eq!(view.recovery(), None, "it reconnects by itself");
        let row = view.row_text(2).expect("the state row");
        assert_eq!(row, "Reconnecting (1/5): The cluster could not be reached.");
    });
}
