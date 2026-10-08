//! A fake session driven through `Connecting`, `AuthRequired`, `Degraded` and `Error`, and the
//! widgets each state shows, in a real cluster tab.

use gpui::TestAppContext;
use oxikube_domain::session::{ClusterSessionState, SessionPhase};
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{ExecInteractivity, HealthSignal};

use super::fixture::{Dispatch, Fixture, Offers};
use super::{id, model::TOKEN};
use crate::connect::ConnectViewModel;

const BODY: &str = "cluster-connect-prod-eu";
const PLACEHOLDER: &str = "cluster-placeholder-prod-eu";

#[gpui::test]
fn connecting_shows_a_spinner_the_server_and_a_cancel_button(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["prod-eu"]);
    fx.begin_connect("prod-eu");
    assert_eq!(fx.phase("prod-eu"), SessionPhase::Connecting);

    for selector in [
        BODY,
        "connect-connecting",
        "connect-spinner",
        "connect-server",
        "connect-context",
        "connect-cancel",
    ] {
        assert!(fx.drawn(selector), "{selector} is drawn while connecting");
    }
    assert!(!fx.drawn("connect-retry"), "nothing to retry yet");
    assert!(
        !fx.drawn(PLACEHOLDER),
        "the cluster's own content is not shown"
    );

    let view = fx.view("prod-eu");
    let model = fx.vcx.update(|_, cx| view.read(cx).model().clone());
    let ConnectViewModel::Connecting(connecting) = model else {
        panic!("{model:?}");
    };
    assert_eq!(
        connecting.server.as_deref(),
        Some("https://prod-eu.example:6443")
    );
    assert_eq!(connecting.context, "prod-eu");
}

#[gpui::test]
fn connecting_gives_way_to_the_cluster_when_the_attempt_succeeds(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["prod-eu"]);
    fx.begin_connect("prod-eu");
    assert!(fx.drawn("connect-spinner"));
    assert_eq!(fx.release(), ClusterSessionState::Ready);
    assert!(!fx.drawn("connect-spinner"));
    assert!(!fx.drawn(BODY));
    assert!(fx.drawn(PLACEHOLDER));
}

#[gpui::test]
fn auth_required_explains_why_and_offers_retry(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["prod-eu"]);
    fx.connector
        .connect_script_for(&id("prod-eu"))
        .push_err(OxiError::auth(
            "the exec plugin reported: token expired",
            false,
        ));
    fx.connect("prod-eu");
    assert_eq!(fx.phase("prod-eu"), SessionPhase::AuthRequired);

    for selector in [
        BODY,
        "connect-auth",
        "connect-auth-message",
        "connect-auth-instructions",
        "connect-policy",
        "connect-open-terminal",
        "connect-retry",
    ] {
        assert!(
            fx.drawn(selector),
            "{selector} is drawn when credentials are needed"
        );
    }
    assert!(!fx.drawn("connect-spinner"));
    assert!(!fx.drawn(PLACEHOLDER));
    let view = fx.view("prod-eu");
    let message = fx.vcx.update(|_, cx| match view.read(cx).model() {
        ConnectViewModel::AuthRequired(auth) => auth.message.summary.clone(),
        other => panic!("{other:?}"),
    });
    assert_eq!(message, "the exec plugin reported: token expired");
}

#[gpui::test]
fn the_exec_policy_line_is_shown_only_when_interaction_is_forbidden(cx: &mut TestAppContext) {
    for (setting, label) in [
        (ExecInteractivity::Never, "Forbid"),
        (ExecInteractivity::IfAvailable, "Ask"),
        (ExecInteractivity::Always, "Allow"),
    ] {
        let mut fx = Fixture::open(cx, &["prod-eu"]);
        fx.sessions
            .open_configured(&crate::catalog::test_support::context("prod-eu"));
        fx.sessions
            .set_exec_interactivity(&id("prod-eu"), setting)
            .expect("open session");
        fx.connector
            .connect_script_for(&id("prod-eu"))
            .push_err(OxiError::auth("token expired", false));
        fx.connect("prod-eu");
        let view = fx.view("prod-eu");
        let (shown, instructions) = fx.vcx.update(|_, cx| match view.read(cx).model() {
            ConnectViewModel::AuthRequired(auth) => (auth.policy.label(), auth.instructions),
            other => panic!("{other:?}"),
        });
        assert_eq!(shown, label);
        // The policy line is drawn only when interaction is really forbidden.
        assert_eq!(
            fx.drawn("connect-policy"),
            setting == ExecInteractivity::Never,
            "{label}"
        );
        assert!(
            !instructions.contains("forbidden"),
            "plain words: {instructions}"
        );
    }
}

#[gpui::test]
fn retrying_after_auth_required_connects_and_shows_the_cluster(cx: &mut TestAppContext) {
    let mut fx = Fixture::start(cx, &["prod-eu"], Dispatch::Run, Offers::default());
    fx.connector
        .connect_script_for(&id("prod-eu"))
        .push_err(OxiError::auth("token expired", false));
    fx.connect("prod-eu");
    assert!(fx.drawn("connect-auth"));

    // The user signs in elsewhere and presses Retry: the next attempt succeeds.
    fx.click("connect-retry");
    assert_eq!(fx.phase("prod-eu"), SessionPhase::Ready);
    assert!(!fx.drawn(BODY), "the connect view gives way to the cluster");
    assert!(!fx.drawn("connect-auth"));
    assert!(fx.drawn(PLACEHOLDER), "the cluster's own content is shown");
}

#[gpui::test]
fn degraded_is_a_banner_above_the_content_that_stays(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["prod-eu"]);
    fx.connect("prod-eu");
    assert!(!fx.drawn("connect-banner"), "no banner while ready");

    fx.connector.report(&id("prod-eu"), HealthSignal::Unhealthy);
    fx.vcx.run_until_parked();
    assert_eq!(fx.phase("prod-eu"), SessionPhase::Degraded);

    let banner = fx.bounds("connect-banner").expect("the banner is drawn");
    let content = fx.bounds(PLACEHOLDER).expect("the content stays");
    assert!(
        banner.bottom() <= content.top() + gpui::px(1.),
        "above the content"
    );
    assert!(fx.drawn("connect-banner-retry"));
    assert!(!fx.drawn(BODY), "not a replacement");

    // Recovered: the banner goes, the content is untouched.
    fx.connector.report(&id("prod-eu"), HealthSignal::Healthy);
    fx.vcx.run_until_parked();
    assert!(!fx.drawn("connect-banner"));
    assert!(fx.drawn(PLACEHOLDER));
}

#[gpui::test]
fn error_shows_a_summary_collapsed_details_and_retry(cx: &mut TestAppContext) {
    let mut fx = Fixture::start(
        cx,
        &["prod-eu"],
        Dispatch::Record,
        Offers {
            terminal: false,
            sources: true,
        },
    );
    fx.connector.script().connect.push_err(OxiError::validation(
        "context `prod-eu` has no cluster entry\nline two",
    ));
    fx.connect("prod-eu");
    assert_eq!(fx.phase("prod-eu"), SessionPhase::Error);

    for selector in [
        BODY,
        "connect-error",
        "connect-error-summary",
        "connect-details-toggle",
        "connect-copy",
        "connect-edit-sources",
        "connect-retry",
    ] {
        assert!(fx.drawn(selector), "{selector} is drawn for an error");
    }
    assert!(!fx.drawn("connect-details"), "details start collapsed");

    fx.click("connect-details-toggle");
    assert!(fx.drawn("connect-details"), "details expand");
    fx.click("connect-details-toggle");
    assert!(!fx.drawn("connect-details"), "and collapse");
}

#[gpui::test]
fn an_error_without_a_sources_page_has_no_sources_link(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["prod-eu"]);
    fx.connector
        .script()
        .connect
        .push_err(OxiError::validation("broken"));
    fx.connect("prod-eu");
    assert!(fx.drawn("connect-error"));
    assert!(!fx.drawn("connect-edit-sources"));
}

#[gpui::test]
fn a_connection_that_dies_shows_the_error_and_a_reconnect_brings_it_back(cx: &mut TestAppContext) {
    let mut fx = Fixture::start(cx, &["prod-eu"], Dispatch::Run, Offers::default());
    fx.connect("prod-eu");
    fx.connector.report(
        &id("prod-eu"),
        HealthSignal::Failed {
            reason: format!("Authorization: Bearer {TOKEN} rejected"),
            kind: ErrorKind::Network,
            retryable: true,
        },
    );
    fx.vcx.run_until_parked();
    assert!(fx.drawn("connect-error"));
    assert!(!fx.drawn(PLACEHOLDER));

    fx.click("connect-retry");
    assert_eq!(fx.phase("prod-eu"), SessionPhase::Ready);
    assert!(fx.drawn(PLACEHOLDER));
}

#[gpui::test]
fn a_connection_lost_to_the_network_says_it_reconnects_by_itself(cx: &mut TestAppContext) {
    let mut fx = Fixture::start(cx, &["prod-eu"], Dispatch::Run, Offers::default());
    fx.connect("prod-eu");
    // The manager schedules its reconnect on the Tokio runtime the adapter reports from. This
    // one is never driven: the schedule stays planned, which is the state the card shows.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    {
        let _inside = runtime.enter();
        fx.connector.report(
            &id("prod-eu"),
            HealthSignal::failed(&OxiError::network("connection reset by peer")),
        );
    }
    fx.vcx.run_until_parked();
    assert_eq!(fx.phase("prod-eu"), SessionPhase::Error);
    assert!(fx.drawn("connect-error"));
    assert!(
        fx.drawn("connect-error-reconnect"),
        "the card promises a reconnect"
    );
    assert!(fx.drawn("connect-retry"), "and Retry still tries at once");

    // Retry takes over: the user's connect ends the schedule.
    fx.click("connect-retry");
    assert_eq!(fx.phase("prod-eu"), SessionPhase::Ready);
    let session = fx.sessions.get(&id("prod-eu")).unwrap();
    assert_eq!(session.auto_reconnect(), None);
    drop(runtime);
}

#[gpui::test]
fn a_permanent_failure_promises_no_reconnect(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["prod-eu"]);
    fx.connect("prod-eu");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    {
        let _inside = runtime.enter();
        fx.connector.report(
            &id("prod-eu"),
            HealthSignal::failed(&OxiError::forbidden("/version is forbidden")),
        );
    }
    fx.vcx.run_until_parked();
    assert!(fx.drawn("connect-error"));
    assert!(!fx.drawn("connect-error-reconnect"));
}

#[gpui::test]
fn revoked_credentials_while_connected_ask_to_sign_in(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["prod-eu"]);
    fx.connect("prod-eu");
    fx.connector.report(
        &id("prod-eu"),
        HealthSignal::failed(&OxiError::auth("Unauthorized", false)),
    );
    fx.vcx.run_until_parked();
    assert_eq!(fx.phase("prod-eu"), SessionPhase::AuthRequired);
    assert!(fx.drawn("connect-auth"));
    assert!(!fx.drawn(PLACEHOLDER));
}

#[gpui::test]
fn a_hidden_cluster_tab_draws_nothing(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["prod-eu", "dev-local"]);
    fx.begin_connect("prod-eu");
    assert!(fx.drawn("connect-spinner"));
    // Another cluster's tab takes the display: the connecting one is not drawn, so its spinner
    // does not run behind the user's back.
    fx.connector.release();
    fx.connect("dev-local");
    assert!(!fx.drawn("connect-spinner"));
    assert!(!fx.drawn(BODY));
}

#[gpui::test]
fn the_connect_view_lives_in_every_tab_and_follows_its_own_cluster(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["prod-eu", "dev-local"]);
    fx.connector
        .connect_script_for(&id("prod-eu"))
        .push_err(OxiError::auth("token expired", false));
    fx.connect("prod-eu");
    fx.connect("dev-local");
    let prod = fx.view("prod-eu");
    let dev = fx.view("dev-local");
    fx.vcx.update(|_, cx| {
        assert!(matches!(
            prod.read(cx).model(),
            ConnectViewModel::AuthRequired(_)
        ));
        assert!(matches!(dev.read(cx).model(), ConnectViewModel::Content));
    });
}
