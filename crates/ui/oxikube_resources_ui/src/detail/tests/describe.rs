//! The Describe tab over `FakeDescribePort`: text, a slow answer, an error and its retry, refresh.

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{DescribeOutput, DescribeSource};
use oxikube_testkit::DescribeCall;

use super::fixture::{Detail, pod_ref, web_pod};
use crate::detail::DescribeState;

fn output(text: &str, source: DescribeSource) -> DescribeOutput {
    DescribeOutput {
        text: text.into(),
        source,
    }
}

const POD_TEXT: &str = "Name:         web-0\nNamespace:    shop\nStatus:       Running\n";

#[gpui::test]
fn describe_starts_when_the_tab_is_first_shown_and_shows_the_text(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    let describe = d.f.ports().describe;
    describe
        .script()
        .describe
        .push_ok(output(POD_TEXT, DescribeSource::Native));
    let view = d.open(&pod_ref("web-0"));
    assert!(
        describe.recorded_calls().is_empty(),
        "the Overview alone describes nothing"
    );
    assert_eq!(
        d.read(&view, |v| v.describe_state().clone()),
        DescribeState::Idle
    );

    d.click("detail-tab-describe");
    assert_eq!(
        describe.recorded_calls(),
        [DescribeCall::Describe(pod_ref("web-0"))]
    );
    assert_eq!(
        d.read(&view, |v| v.describe_state().clone()),
        DescribeState::Ready {
            source: DescribeSource::Native
        }
    );
    assert_eq!(
        d.read(&view, |v| v.describe_text().map(str::to_owned))
            .as_deref(),
        Some(POD_TEXT)
    );
    assert!(d.shown("detail-describe-text"));
    assert!(d.shown("describe-source"));
    let code = d
        .read(&view, |v| v.describe.view.clone())
        .expect("a code view");
    let (text, language) = d.f.vcx.update(|_, cx| {
        let code = code.read(cx);
        (code.text().map(|t| t.to_string()), code.language())
    });
    assert_eq!(
        text.as_deref(),
        Some(POD_TEXT),
        "laid out off the UI thread"
    );
    assert_eq!(language, None, "plain text");

    // Back and forth between tabs does not describe again.
    d.click("detail-tab-overview");
    d.click("detail-tab-describe");
    assert_eq!(describe.recorded_calls().len(), 1);
}

#[gpui::test]
fn a_slow_describe_shows_a_spinner_until_the_answer_arrives(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    let describe = d.f.ports().describe;
    describe.hold();
    let view = d.open(&pod_ref("web-0"));
    d.click("detail-tab-describe");
    assert_eq!(
        d.read(&view, |v| v.describe_state().clone()),
        DescribeState::Loading
    );
    assert_eq!(describe.held(), 1, "the call is in flight");
    assert!(
        d.shown("describe-loading"),
        "a spinner stands in for the text"
    );
    assert!(!d.shown("detail-describe-text"));

    describe
        .script()
        .describe
        .push_ok(output(POD_TEXT, DescribeSource::Native));
    describe.release();
    d.settle();
    assert!(matches!(
        d.read(&view, |v| v.describe_state().clone()),
        DescribeState::Ready { .. }
    ));
    assert!(!d.shown("describe-loading"));
    assert!(d.shown("detail-describe-text"));
}

#[gpui::test]
fn an_error_shows_its_message_and_retry_describes_again(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    let describe = d.f.ports().describe;
    describe.script().describe.push_err(OxiError::unsupported(
        "native describe does not cover Widget; kubectl was not found",
    ));
    let view = d.open(&pod_ref("web-0"));
    d.click("detail-tab-describe");
    match d.read(&view, |v| v.describe_state().clone()) {
        DescribeState::Failed { kind, message } => {
            assert_eq!(kind, ErrorKind::Unsupported);
            assert!(message.contains("kubectl was not found"), "{message}");
        }
        other => panic!("expected a failure, got {other:?}"),
    }
    assert!(
        d.shown("describe-error"),
        "an error state, not an empty pane"
    );
    assert!(d.shown("describe-error-message"));
    assert!(!d.shown("detail-describe-text"));

    // Retry: the command, then a second describe that works.
    describe
        .script()
        .describe
        .push_ok(output(POD_TEXT, DescribeSource::KubectlFallback));
    d.f.dispatcher.clear();
    d.click("describe-retry");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::ResourceRefreshDescribe {
            target: pod_ref("web-0")
        }]
    );
    assert_eq!(describe.recorded_calls().len(), 2);
    assert_eq!(
        d.read(&view, |v| v.describe_state().clone()),
        DescribeState::Ready {
            source: DescribeSource::KubectlFallback
        }
    );
    assert!(!d.shown("describe-error"));
    assert!(d.shown("detail-describe-text"));
}

#[gpui::test]
fn refresh_keeps_the_text_on_screen_and_replaces_it_with_the_new_answer(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    let describe = d.f.ports().describe;
    describe
        .script()
        .describe
        .push_ok(output("Status: Pending\n", DescribeSource::Native));
    let view = d.open(&pod_ref("web-0"));
    d.click("detail-tab-describe");

    describe.hold();
    d.click("describe-refresh");
    assert_eq!(
        d.read(&view, |v| v.describe_state().clone()),
        DescribeState::Loading
    );
    assert_eq!(
        d.read(&view, |v| v.describe_text().map(str::to_owned))
            .as_deref(),
        Some("Status: Pending\n"),
        "the previous text stays while it refreshes"
    );
    assert!(d.shown("detail-describe-text"));

    describe
        .script()
        .describe
        .push_ok(output("Status: Running\n", DescribeSource::Native));
    describe.release();
    d.settle();
    d.draw();
    assert_eq!(
        d.read(&view, |v| v.describe_text().map(str::to_owned))
            .as_deref(),
        Some("Status: Running\n")
    );
    let code = d.read(&view, |v| v.describe.view.clone()).unwrap();
    let shown =
        d.f.vcx
            .update(|_, cx| code.read(cx).text().map(|t| t.to_string()))
            .unwrap_or_default();
    assert_eq!(
        shown, "Status: Running\n",
        "the view was given the new text"
    );
}

#[gpui::test]
fn a_failed_refresh_keeps_the_old_text_under_a_notice(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    let describe = d.f.ports().describe;
    describe
        .script()
        .describe
        .push_ok(output(POD_TEXT, DescribeSource::Native));
    let view = d.open(&pod_ref("web-0"));
    d.click("detail-tab-describe");
    describe
        .script()
        .describe
        .push_err(OxiError::network("connection refused"));
    d.click("describe-refresh");
    assert!(matches!(
        d.read(&view, |v| v.describe_state().clone()),
        DescribeState::Failed {
            kind: ErrorKind::Network,
            ..
        }
    ));
    assert!(d.shown("describe-refresh-error"));
    assert!(
        d.shown("detail-describe-text"),
        "the last good text is still there"
    );
}

#[gpui::test]
fn only_the_latest_request_is_applied(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    let describe = d.f.ports().describe;
    describe.hold();
    let view = d.open(&pod_ref("web-0"));
    d.click("detail-tab-describe");
    d.update(&view, |v, cx| v.refresh_describe(cx));
    assert_eq!(
        describe.held(),
        1,
        "the first request was cancelled when the second began"
    );

    describe
        .script()
        .describe
        .push_ok(output("second\n", DescribeSource::Native));
    describe.release();
    d.settle();
    assert_eq!(
        d.read(&view, |v| v.describe_text().map(str::to_owned))
            .as_deref(),
        Some("second\n")
    );
}

#[gpui::test]
fn the_source_of_the_text_is_named(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    d.f.ports()
        .describe
        .script()
        .describe
        .push_ok(output(POD_TEXT, DescribeSource::KubectlFallback));
    let view = d.open(&pod_ref("web-0"));
    d.click("detail-tab-describe");
    assert_eq!(
        d.read(&view, |v| v.describe_state().clone()),
        DescribeState::Ready {
            source: DescribeSource::KubectlFallback
        }
    );
}
