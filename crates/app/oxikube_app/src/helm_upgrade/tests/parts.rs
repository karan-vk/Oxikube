//! The pieces: the spec and its command, the commands' declarations, the release hand-off.

use std::time::Duration;

use oxikube_domain::command::{Command, CommandId, lookup};
use oxikube_domain::helm::ChartRef;
use oxikube_domain::safety::{ConfirmTier, Risk};

use super::{spec, web};
use crate::helm_upgrade::{UpgradeRequests, UpgradeSpec, UpgradeTarget};

#[test]
fn a_spec_round_trips_through_its_command() {
    let mut original = spec();
    original.reuse_values = true;
    original.wait = true;
    original.timeout = Some(Duration::from_secs(120));
    assert_eq!(UpgradeSpec::of(&original.command()), Some(original.clone()));
    let latest = UpgradeSpec::new(web(), ChartRef::new("nginx", ""));
    assert_eq!(UpgradeSpec::of(&latest.command()), Some(latest.clone()));
    let request = original.request(true).unwrap();
    assert!(request.reuse_values && request.dry_run && request.wait);
    assert_eq!(request.timeout, Duration::from_secs(120));
    assert_eq!(
        latest.request(false).unwrap().timeout,
        oxikube_domain::helm::request::DEFAULT_TIMEOUT
    );
    assert!(!format!("{original:?}").contains("hunter2"));
    assert_eq!(UpgradeSpec::of(&Command::HelmOpenCharts), None);
}

#[test]
fn upgrade_is_a_medium_mutation_and_opening_the_tab_is_a_read() {
    let upgrade = lookup(CommandId::HELM_UPGRADE).expect("declared");
    assert!(upgrade.mutating);
    assert_eq!(upgrade.risk, Some(Risk::Medium));
    assert_eq!(upgrade.confirm, ConfirmTier::Simple);
    assert_eq!(CommandId::HELM_UPGRADE.tool_name(), "helm.upgrade");
    let open = lookup(CommandId::HELM_UPGRADE_RELEASE).expect("declared");
    assert!(!open.mutating);
    assert_eq!(
        CommandId::HELM_UPGRADE_RELEASE.tool_name(),
        "helm.upgrade_release"
    );
}

#[test]
fn the_guard_text_names_the_release_the_chart_and_the_flag_never_the_values() {
    let mut s = spec();
    s.reuse_values = true;
    let command = s.command();
    let summary = crate::helm_upgrade::summary(&command, "kind-a").unwrap();
    assert_eq!(
        summary,
        "Upgrade release web in namespace demo to chart bitnami/nginx 15.4.0, reusing the \
         previous values on kind-a"
    );
    let detail = crate::helm_upgrade::audit_detail(&command).unwrap();
    assert_eq!(
        detail,
        "chart=bitnami/nginx version=15.4.0 reuse_values=true wait=false values=given"
    );
    assert!(!detail.contains("hunter2"));
    assert!(crate::helm_upgrade::audit_detail(&Command::HelmOpenCharts).is_none());
}

#[tokio::test]
async fn opening_the_tab_hands_the_release_over_and_changes_nothing() {
    let s = super::Setup::new(false);
    let mut rx = s.requests.subscribe();
    let command = Command::HelmUpgradeRelease {
        namespace: Some("demo".into()),
        release: Some("web".into()),
    };
    let outcome =
        s.h.dispatch(
            command,
            crate::testing::ctx(oxikube_domain::audit::Initiator::Ui),
        )
        .unwrap();
    assert!(matches!(outcome, crate::command_bus::Outcome::Completed(_)));
    rx.changed().await.unwrap();
    assert_eq!(
        *rx.borrow(),
        Some(UpgradeTarget {
            release: Some(("demo".into(), "web".into())),
            chart: None,
        })
    );
    assert!(s.helm.recorded_calls().is_empty());
    // Without a release the tab lists them.
    let command = Command::HelmUpgradeRelease {
        namespace: None,
        release: None,
    };
    s.h.dispatch(
        command,
        crate::testing::ctx(oxikube_domain::audit::Initiator::Ui),
    )
    .unwrap();
    rx.changed().await.unwrap();
    assert_eq!(rx.borrow().as_ref().unwrap().release, None);
    let _ = UpgradeRequests::default();
}
