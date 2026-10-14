//! The other sources (E14-S07) through `NotificationSourcePort`: each produces one notification
//! with its dedupe key and level, repeats update instead of duplicating, the agent permission one
//! is resolved by the answer, and the kind switch and the cluster mute suppress them.

use std::sync::Arc;

use futures::StreamExt as _;
use oxikube_domain::notification::{
    AgentPermissionRequest, ForwardPhase, NotificationKind, UpdateAvailable,
};
use oxikube_ports::{NotificationLevel, NotificationSourcePort};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeNotifierPort,
    FakeStatePort, agent_permission, emit_agent_permission, emit_portforward_drop,
    emit_portforward_recovery, emit_update_available, hook_cluster, portforward_change,
};

use super::{cluster, input, secs, t0};
use crate::events::{EventService, EventsConfig};
use crate::notifications::{NotificationService, forward_to_os};
use crate::os_notifications::OsNotificationService;
use crate::session::ClusterSessionManager;

/// A service with no sessions and no spawner: only the submit path runs, on a manual clock.
fn service() -> (NotificationService, Arc<FakeClockPort>) {
    let clock = Arc::new(FakeClockPort::new(t0()));
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
    );
    let events = EventService::new(sessions.clone(), clock.clone(), EventsConfig::default());
    let service = NotificationService::new(
        events,
        sessions,
        Arc::new(FakeStatePort::new()),
        clock.clone(),
    );
    (service, clock)
}

fn keys(service: &NotificationService) -> Vec<String> {
    service
        .list()
        .iter()
        .map(|n| n.dedupe_key.to_string())
        .collect()
}

#[test]
fn a_port_forward_drop_is_one_row_per_forward_with_its_key() {
    let (service, _) = service();
    let hooks = service.sources();
    emit_portforward_drop(&*hooks, "fwd-1");

    let rows = service.list();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.kind, NotificationKind::PortForward);
    assert_eq!(row.level, NotificationLevel::Warning);
    assert_eq!(row.cluster, Some(hook_cluster()));
    assert_eq!(
        &*row.dedupe_key,
        format!("portforward:{}:fwd-1:dropped", hook_cluster())
    );
    assert!(row.title.contains("pod/fwd-1 8080:80"), "{}", row.title);
    assert_eq!(row.body, "connection reset");
    assert!(!row.read);

    // The same forward dropping again updates the row; another forward adds one.
    emit_portforward_drop(&*hooks, "fwd-1");
    emit_portforward_drop(&*hooks, "fwd-2");
    let rows = service.list();
    assert_eq!(rows.len(), 2);
    let again = rows
        .iter()
        .find(|n| n.dedupe_key.contains("fwd-1"))
        .expect("the first forward's row");
    assert_eq!(again.count, 2);
}

#[test]
fn giving_up_escalates_the_same_row_and_a_recovery_marks_it_read() {
    let (service, _) = service();
    let hooks = service.sources();
    emit_portforward_drop(&*hooks, "fwd-1");
    hooks.port_forward_changed(portforward_change(
        "fwd-1",
        ForwardPhase::Reconnecting,
        ForwardPhase::Failed,
    ));
    let rows = service.list();
    assert_eq!(rows.len(), 1, "one row per forward");
    assert_eq!(rows[0].level, NotificationLevel::Error);
    assert!(
        rows[0].title.starts_with("Port-forward failed"),
        "{}",
        rows[0].title
    );
    assert_eq!(rows[0].count, 2);

    hooks.port_forward_changed(portforward_change(
        "fwd-1",
        ForwardPhase::Failed,
        ForwardPhase::Active,
    ));
    assert!(service.list()[0].read, "the forward is back");
    assert_eq!(service.unread(), 0);
}

#[tokio::test]
async fn giving_up_is_raised_again_although_the_drop_just_was() {
    let (service, clock) = service();
    let hooks = service.sources();
    let mut sub = service.subscribe();
    sub.next().await.expect("first item");

    emit_portforward_drop(&*hooks, "fwd-1");
    let drop = sub.next().await.expect("the drop");
    assert_eq!(drop.raised.len(), 1);
    assert_eq!(drop.raised[0].level, NotificationLevel::Warning);

    // Seconds later, far inside the per-row interval: it failed for good.
    clock.advance(secs(20));
    hooks.port_forward_changed(portforward_change(
        "fwd-1",
        ForwardPhase::Reconnecting,
        ForwardPhase::Failed,
    ));
    let failed = sub.next().await.expect("the failure");
    assert_eq!(failed.raised.len(), 1, "the worse news is told");
    assert_eq!(failed.raised[0].level, NotificationLevel::Error);
    assert!(failed.raised[0].title.starts_with("Port-forward failed"));

    // The same level again is still quiet.
    hooks.port_forward_changed(portforward_change(
        "fwd-1",
        ForwardPhase::Reconnecting,
        ForwardPhase::Failed,
    ));
    let again = sub.next().await.expect("the repeat");
    assert!(again.raised.is_empty(), "{:?}", again.raised);
}

#[test]
fn changes_that_are_not_drops_say_nothing() {
    let (service, _) = service();
    let hooks = service.sources();
    for (from, to) in [
        (ForwardPhase::Starting, ForwardPhase::Active),
        (ForwardPhase::Starting, ForwardPhase::Failed),
        (ForwardPhase::Active, ForwardPhase::Stopped),
        (ForwardPhase::Reconnecting, ForwardPhase::Active),
    ] {
        hooks.port_forward_changed(portforward_change("fwd-1", from, to));
    }
    emit_portforward_recovery(&*hooks, "fwd-9");
    assert!(service.list().is_empty());
}

#[test]
fn an_update_notifies_once_per_version_without_a_cluster() {
    let (service, _) = service();
    let hooks = service.sources();
    emit_update_available(&*hooks, "0.6.1");
    emit_update_available(&*hooks, "0.6.1");
    emit_update_available(&*hooks, "0.6.1");
    let rows = service.list();
    assert_eq!(rows.len(), 1, "reported three times, announced once");
    let row = &rows[0];
    assert_eq!(row.kind, NotificationKind::UpdateAvailable);
    assert_eq!(row.level, NotificationLevel::Info);
    assert_eq!(&*row.dedupe_key, "update:0.6.1");
    assert_eq!(row.cluster, None);
    assert_eq!(row.count, 1);
    assert_eq!(
        row.link.as_deref(),
        Some("https://example.com/oxikube/releases/0.6.1")
    );
    assert!(
        row.to_os().cluster.is_none(),
        "the OS notification names no cluster"
    );

    emit_update_available(&*hooks, "0.7.0");
    assert_eq!(keys(&service), ["update:0.7.0", "update:0.6.1"]);
}

#[test]
fn an_update_is_not_announced_again_after_its_row_scrolled_out() {
    let (service, clock) = service();
    let hooks = service.sources();
    emit_update_available(&*hooks, "1.0.0");
    // Longer than the dedupe window: a plain repeat would add a new row.
    clock.advance(secs(3_600));
    emit_update_available(&*hooks, "1.0.0");
    assert_eq!(service.list().len(), 1);
}

#[test]
fn only_https_links_are_kept() {
    let (service, _) = service();
    for (i, link) in [
        "http://example.com",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "https://",
        "https://exa mple.com",
    ]
    .into_iter()
    .enumerate()
    {
        service.update_available(UpdateAvailable {
            version: format!("9.{i}"),
            link: Some(link.to_owned()),
            summary: None,
        });
    }
    let rows = service.list();
    assert_eq!(rows.len(), 5);
    assert!(rows.iter().all(|n| n.link.is_none()), "{rows:?}");
    assert!(rows[0].body.contains("is ready to download"));
}

#[test]
fn an_agent_permission_request_stays_until_it_is_answered() {
    let (service, _) = service();
    let hooks = service.sources();
    emit_agent_permission(&*hooks, "s1", "r1");
    let rows = service.list();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.kind, NotificationKind::AgentPermission);
    assert_eq!(&*row.dedupe_key, "agent-permission:s1:r1");
    assert_eq!(row.title, "Claude needs permission");
    assert!(row.sticky());
    assert!(!row.read);

    // Another request is another row; answering one resolves only that one.
    emit_agent_permission(&*hooks, "s1", "r2");
    hooks.agent_permission_resolved("s1", "r1");
    assert_eq!(service.unread(), 1);
    let rows = service.list();
    assert!(
        rows.iter()
            .find(|n| n.dedupe_key.ends_with("r1"))
            .unwrap()
            .read
    );
    assert!(
        !rows
            .iter()
            .find(|n| n.dedupe_key.ends_with("r2"))
            .unwrap()
            .read
    );

    // Answering something unknown is harmless.
    hooks.agent_permission_resolved("s9", "nope");
    assert_eq!(service.unread(), 1);
}

#[test]
fn no_notification_while_the_agent_panel_is_focused() {
    let (service, _) = service();
    service.agent_permission_requested(AgentPermissionRequest {
        panel_focused: true,
        ..agent_permission("s1", "r1")
    });
    assert!(service.list().is_empty());
}

#[tokio::test]
async fn a_permission_request_is_raised_even_when_warnings_used_the_raise_limit() {
    let (service, _) = service();
    let mut sub = service.subscribe();
    let first = sub.next().await.expect("first item");
    assert!(first.changed);
    for i in 0..10 {
        service.submit(input(&cluster(), &format!("warning-{i}")));
    }
    let burst = sub.next().await.expect("burst");
    assert_eq!(
        burst.raised.len(),
        5,
        "the overall limit holds for warnings"
    );
    emit_agent_permission(&*service.sources(), "s1", "r1");
    let next = sub.next().await.expect("the request");
    assert_eq!(next.raised.len(), 1);
    assert_eq!(next.raised[0].kind, NotificationKind::AgentPermission);
}

#[test]
fn a_kind_switched_off_adds_nothing_and_back_on_adds_again() {
    let (service, _) = service();
    let hooks = service.sources();
    assert!(service.is_kind_enabled(NotificationKind::UpdateAvailable));
    assert!(service.set_kind_enabled(NotificationKind::UpdateAvailable, false));
    assert!(
        !service.set_kind_enabled(NotificationKind::UpdateAvailable, false),
        "already off"
    );
    emit_update_available(&*hooks, "2.0.0");
    emit_portforward_drop(&*hooks, "fwd-1");
    assert_eq!(
        keys(&service).len(),
        1,
        "only the forward: {:?}",
        keys(&service)
    );

    // The version was not used up while the kind was off.
    service.set_kind_enabled(NotificationKind::UpdateAvailable, true);
    emit_update_available(&*hooks, "2.0.0");
    assert!(keys(&service).contains(&"update:2.0.0".to_owned()));

    service.set_kind_enabled(NotificationKind::PortForward, false);
    emit_portforward_drop(&*hooks, "fwd-2");
    service.set_kind_enabled(NotificationKind::AgentPermission, false);
    emit_agent_permission(&*hooks, "s1", "r1");
    assert_eq!(service.list().len(), 2, "{:?}", keys(&service));
}

#[tokio::test]
async fn muting_a_cluster_silences_its_forwards_and_requests_but_not_updates() {
    let (service, _) = service();
    let hooks = service.sources();
    service.mute(&hook_cluster()).await.expect("muted");
    emit_portforward_drop(&*hooks, "fwd-1");
    emit_agent_permission(&*hooks, "s1", "r1");
    assert!(service.list().is_empty());
    // A request that concerns no cluster, and an update, are not about the muted cluster.
    service.agent_permission_requested(AgentPermissionRequest {
        cluster: None,
        ..agent_permission("s1", "r2")
    });
    emit_update_available(&*hooks, "3.0.0");
    assert_eq!(service.list().len(), 2);

    service.unmute(&hook_cluster()).await.expect("unmuted");
    emit_portforward_drop(&*hooks, "fwd-1");
    assert_eq!(service.list().len(), 3);
}

#[tokio::test]
async fn raised_sources_reach_the_os_with_their_dedupe_key_as_the_tag() {
    let (service, _) = service();
    let notifier = Arc::new(FakeNotifierPort::new());
    let os = OsNotificationService::new(notifier.clone());
    os.apply(true).await;
    let forward = tokio::spawn(forward_to_os(service.clone(), os));
    tokio::task::yield_now().await;

    emit_update_available(&*service.sources(), "4.0.0");
    emit_agent_permission(&*service.sources(), "s1", "r1");
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while notifier.shown().len() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("both reached the OS");
    forward.abort();
    let shown = notifier.shown();
    let tags: Vec<_> = shown.iter().filter_map(|n| n.tag.clone()).collect();
    assert!(tags.contains(&"update:4.0.0".to_owned()), "{tags:?}");
    assert!(
        tags.contains(&"agent-permission:s1:r1".to_owned()),
        "{tags:?}"
    );
    assert!(shown.iter().all(|n| n.action_target.is_none()));
}
