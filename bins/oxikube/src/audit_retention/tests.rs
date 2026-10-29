//! The setting reaches the scheduler, and the scheduler prunes the state database.

use std::time::Duration;

use gpui::{BorrowAppContext as _, TestAppContext};
use oxikube_app::audit::MAX_RETENTION_DAYS;
use oxikube_domain::audit::{AuditOutcome, AuditRecord, Initiator};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_ports::{ClockPort, StatePort};
use oxikube_settings::SettingsStore;
use oxikube_testkit::TestPorts;

use super::{AuditRetentionWiring, start};
use crate::app_state::AppState;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

fn edit_settings_file(cx: &mut TestAppContext, text: &str) {
    let text = text.to_owned();
    cx.update(|cx| {
        cx.update_global::<SettingsStore, _>(|store, _| store.set_user_settings(&text).unwrap());
    });
    cx.run_until_parked();
}

fn record_aged(ports: &TestPorts, days: u32) -> AuditRecord {
    let target = ResourceRef::namespaced(
        ClusterId::new("kubeconfig", &ContextName::new("prod")),
        Gvk::from_api_version("v1", "Pod"),
        "default",
        format!("pod-{days}"),
    );
    AuditRecord::new(
        ports.clock.now() - DAY * days,
        "me",
        Initiator::Ui,
        "pod::Delete",
        target,
        false,
        AuditOutcome::Succeeded,
    )
}

fn kept(ports: &TestPorts) -> usize {
    ports.state.audit_log().len()
}

#[gpui::test]
fn the_audit_log_is_pruned_at_start_with_the_default_window(cx: &mut TestAppContext) {
    let ports = TestPorts::empty();
    futures::executor::block_on(ports.state.append_audit(&[
        record_aged(&ports, 120),
        record_aged(&ports, 100),
        record_aged(&ports, 10),
    ]))
    .unwrap();
    cx.update(|cx| {
        AppState::test_with(cx, &ports);
        start(cx);
    });
    cx.run_until_parked();
    assert_eq!(kept(&ports), 1, "default.json keeps 90 days");
}

#[gpui::test]
fn editing_retention_days_applies_at_once_and_zero_keeps_everything(cx: &mut TestAppContext) {
    let ports = TestPorts::empty();
    futures::executor::block_on(
        ports
            .state
            .append_audit(&[record_aged(&ports, 40), record_aged(&ports, 10)]),
    )
    .unwrap();
    cx.update(|cx| {
        AppState::test_with(cx, &ports);
        start(cx);
    });
    cx.run_until_parked();
    assert_eq!(kept(&ports), 2);

    edit_settings_file(cx, r#"{ "audit": { "retention_days": 30 } }"#);
    assert_eq!(kept(&ports), 1);

    edit_settings_file(cx, r#"{ "audit": { "retention_days": 0 } }"#);
    let days = cx.update(|cx| cx.global::<AuditRetentionWiring>().retention().days());
    assert_eq!(days, 0);
    assert_eq!(kept(&ports), 1);
}

#[test]
fn both_layers_clamp_at_the_same_ceiling() {
    assert_eq!(
        MAX_RETENTION_DAYS,
        oxikube_settings::audit::MAX_RETENTION_DAYS
    );
}
