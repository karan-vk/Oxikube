//! The empty, no-match, loading and unavailable states.

use gpui::TestAppContext;
use oxikube_app::audit::query::AuditFilters;
use oxikube_domain::OxiError;
use oxikube_domain::audit::Initiator;

use super::{Fx, seed};

#[gpui::test]
fn an_empty_log_says_so(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx, Vec::new());
    fx.open();
    assert!(fx.drawn("audit-empty"));
    assert!(!fx.drawn("audit-table"));
}

#[gpui::test]
fn filters_with_no_match_offer_to_clear_them(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx, seed(3));
    fx.open();
    fx.with_view(|view, cx| {
        view.set_filters(
            AuditFilters {
                initiator: Some(Initiator::Plugin),
                ..AuditFilters::default()
            },
            cx,
        );
    });
    assert!(fx.drawn("audit-no-match"));
    fx.click("audit-clear-empty");
    assert!(fx.drawn("audit-table"));
    assert_eq!(fx.names().len(), 3);
}

#[gpui::test]
fn an_unavailable_database_is_a_warning_with_retry(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx, seed(3));
    fx.state
        .script()
        .query_audit
        .push_err(OxiError::internal("database is locked"));
    fx.open();
    assert!(fx.drawn("audit-unavailable"));
    assert_eq!(
        fx.with_view(|v, _| v.error().map(str::to_owned)).as_deref(),
        Some("database is locked")
    );
    fx.click("audit-retry");
    assert!(fx.drawn("audit-table"), "the retry read the log");
    assert!(fx.with_view(|v, _| v.error().is_none()));
}

#[gpui::test]
fn a_failing_read_after_rows_are_shown_keeps_them_under_a_banner(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx, seed(3));
    fx.open();
    fx.state
        .script()
        .query_audit
        .push_err(OxiError::internal("disk I/O error"));
    fx.with_view(|view, cx| view.reload(cx));
    assert!(fx.drawn("audit-banner"));
    assert_eq!(fx.names().len(), 3);
}
