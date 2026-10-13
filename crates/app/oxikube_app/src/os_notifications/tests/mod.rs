//! Tests of the OS notification service and its commands, over `FakeNotifierPort`.

mod commands;
mod service;

use std::sync::Arc;

use oxikube_ports::NotificationLevel;
use oxikube_testkit::FakeNotifierPort;

use super::{ClickAction, OsNotificationService};
use oxikube_ports::Notification;

pub(super) fn service() -> (OsNotificationService, Arc<FakeNotifierPort>) {
    let notifier = Arc::new(FakeNotifierPort::new());
    (OsNotificationService::new(notifier.clone()), notifier)
}

pub(super) fn warning(text: &str) -> Notification {
    Notification::new("Deployment api", text, NotificationLevel::Warning)
}

pub(super) fn target() -> oxikube_domain::ids::ResourceRef {
    "3f2a9c01b7d4e865/apps/v1/Deployment/prod/api"
        .parse()
        .unwrap()
}

#[test]
fn a_click_selects_the_cluster_then_opens_the_object() {
    use oxikube_domain::command::Command;
    let action = ClickAction {
        cluster: Some(target().cluster),
        open: Some(target()),
    };
    assert_eq!(
        action.commands(),
        vec![
            Command::ClusterSelect {
                cluster: target().cluster
            },
            Command::ResourceOpen { target: target() },
        ]
    );
    // The object alone is enough to name the cluster.
    let action = ClickAction {
        cluster: None,
        open: Some(target()),
    };
    assert_eq!(action.commands().len(), 2);
    // The test notification names nothing: the click only focuses the window.
    let nothing = ClickAction {
        cluster: None,
        open: None,
    };
    assert!(nothing.commands().is_empty());
}
