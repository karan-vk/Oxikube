//! The `AuthRequired` path: classified by the adapter, retried by the user.

use oxikube_domain::session::{ClusterSessionState, SessionPhase};
use oxikube_domain::{Capabilities, OxiError};

use super::{Harness, id};

const LOGIN: &str = "exec plugin `aws` needs a login: run `aws sso login --profile dev`";

#[test]
fn connector_auth_failure_requires_auth_then_retry_succeeds() {
    let mut h = Harness::new();
    h.connector
        .script()
        .connect
        .push_err(OxiError::auth(LOGIN, false));

    let state = h.connect("a");
    assert_eq!(
        state,
        ClusterSessionState::AuthRequired {
            reason: LOGIN.into()
        }
    );
    let session = h.manager.get(&id("a")).unwrap();
    assert_eq!(session.state().reason(), Some(LOGIN));
    assert!(session.resources().is_none());
    assert_eq!(session.capabilities(), Capabilities::empty());
    // Auth is never retried automatically: one connector call, no backoff sleep.
    assert_eq!(h.connector.recorded_calls().len(), 1);
    assert!(h.clock.recorded_calls().is_empty());

    // The user signed in; connecting again goes AuthRequired -> Connecting -> Ready.
    assert_eq!(h.connect("a"), ClusterSessionState::Ready);
    assert_eq!(
        h.phases("a"),
        [
            SessionPhase::Connecting,
            SessionPhase::AuthRequired,
            SessionPhase::Connecting,
            SessionPhase::Ready
        ]
    );
}

#[test]
fn a_401_during_discovery_requires_auth() {
    let h = Harness::new();
    h.connector
        .ports_for(&id("a"))
        .discovery
        .script()
        .discover
        .push_err(OxiError::auth("401 Unauthorized: token expired", true));
    assert_eq!(h.connect("a").phase(), SessionPhase::AuthRequired);
    // The half-made connection was dropped.
    assert_eq!(h.connector.live_connections(&id("a")), 0);
}

#[test]
fn an_auth_failure_of_the_capability_probe_requires_auth() {
    let h = Harness::new();
    h.connector
        .ports_for(&id("a"))
        .access
        .script()
        .capabilities
        .push_err(OxiError::auth("token revoked", false));
    assert_eq!(h.connect("a").phase(), SessionPhase::AuthRequired);
}

#[test]
fn giving_up_on_auth_disconnects() {
    let mut h = Harness::new();
    h.connector
        .script()
        .connect
        .push_err(OxiError::auth(LOGIN, false));
    h.connect("a");
    h.drain();
    h.manager.disconnect(&id("a")).unwrap();
    assert_eq!(h.phases("a"), [SessionPhase::Disconnected]);
    // A second disconnect is a no-op, not an error.
    h.manager.disconnect(&id("a")).unwrap();
    assert!(h.drain().is_empty());
}

#[test]
fn reasons_are_redacted() {
    let h = Harness::new();
    h.connector.script().connect.push_err(OxiError::auth(
        "exec plugin printed Authorization: Bearer abcdefghijklmnopqrstuvwxyz0123456789",
        false,
    ));
    let state = h.connect("a");
    let reason = state.reason().unwrap();
    assert!(
        !reason.contains("abcdefghijklmnopqrstuvwxyz0123456789"),
        "{reason}"
    );
}
