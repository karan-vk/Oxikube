//! Error text is redacted wherever it comes from: a fake bearer token never reaches the model,
//! the details, or the clipboard.

use gpui::TestAppContext;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::HealthSignal;

use super::fixture::Fixture;
use super::id;
use super::model::TOKEN;
use crate::connect::ConnectViewModel;

fn leaky() -> String {
    format!("exec plugin stderr: Authorization: Bearer {TOKEN}\nretry with token={TOKEN}")
}

fn shown(fx: &mut Fixture, name: &str) -> String {
    let view = fx.view(name);
    fx.vcx
        .update(|_, cx| format!("{:?}", view.read(cx).model()))
}

fn copied(fx: &mut Fixture) -> String {
    fx.click("connect-copy");
    fx.vcx
        .update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
        .expect("something was copied")
}

#[gpui::test]
fn a_token_in_an_auth_message_is_never_shown_or_copied(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["prod-eu"]);
    fx.connector
        .connect_script_for(&id("prod-eu"))
        .push_err(OxiError::auth(leaky(), false));
    fx.connect("prod-eu");
    assert!(fx.drawn("connect-auth-message"));
    let model = shown(&mut fx, "prod-eu");
    assert!(model.contains("AuthRequired"), "{model}");
    assert!(!model.contains(TOKEN), "{model}");
    // The message is long enough to be summarised: its details are copyable.
    let text = copied(&mut fx);
    assert!(!text.contains(TOKEN), "{text}");
    assert!(
        text.contains("exec plugin stderr"),
        "the rest of the message survives: {text}"
    );
}

#[gpui::test]
fn a_token_in_a_connection_error_is_never_shown_or_copied(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["prod-eu"]);
    fx.connector
        .script()
        .connect
        .push_err(OxiError::validation(leaky()));
    fx.connect("prod-eu");
    let model = shown(&mut fx, "prod-eu");
    assert!(!model.contains(TOKEN), "{model}");
    fx.click("connect-details-toggle");
    assert!(fx.drawn("connect-details"));
    let text = copied(&mut fx);
    assert!(!text.contains(TOKEN), "{text}");
    assert!(text.contains("exec plugin stderr"), "{text}");
}

#[gpui::test]
fn a_token_in_a_health_failure_is_never_shown(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["prod-eu"]);
    fx.connect("prod-eu");
    fx.connector.report(
        &id("prod-eu"),
        HealthSignal::Failed {
            reason: leaky(),
            kind: ErrorKind::Network,
            retryable: true,
        },
    );
    fx.vcx.run_until_parked();
    let model = shown(&mut fx, "prod-eu");
    assert!(!model.contains(TOKEN), "{model}");
    let view = fx.view("prod-eu");
    fx.vcx.update(|_, cx| {
        let ConnectViewModel::Error(error) = view.read(cx).model() else {
            panic!("error expected");
        };
        assert!(!error.details.contains(TOKEN));
        assert!(!error.message.summary.contains(TOKEN));
        assert!(!error.message.full.contains(TOKEN));
    });
}
