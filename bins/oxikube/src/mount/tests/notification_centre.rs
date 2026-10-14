//! The notification centre in the running app (E14-S05; the other sources, E14-S07), over
//! `TestPorts`: a new Warning event of a connected cluster toasts and lights the status-bar bell (the backlog does not), the key opens
//! the centre in the right dock with the row, the five commands are on the bus with their tool
//! stubs, and a muted cluster is remembered in the state db.

use std::time::Duration;

use gpui::{BorrowAppContext as _, TestAppContext};
use oxikube_domain::command::{Command, CommandId};
use oxikube_ports::{StateKey, StatePort as _};
use oxikube_testkit::{
    FakeEventFeed, TICK, TestPorts, emit_agent_permission, emit_portforward_drop,
    emit_update_available, event,
};
use oxikube_workspace::DockPosition;
use oxikube_workspace::notifications::NotificationPanel;

use super::App;
use crate::app_state::AppState;

fn settle(app: &mut App) {
    app.vcx.executor().advance_clock(Duration::from_secs(1));
    app.vcx.run_until_parked();
}

fn toggle_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-shift-i"
    } else {
        "ctrl-shift-i"
    }
}

#[gpui::test]
fn a_new_warning_toasts_lights_the_bell_and_the_key_opens_the_centre(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    let cluster = TestPorts::cluster_id();
    let events = ports.connector.ports_for(&cluster).events;
    events.install(
        FakeEventFeed::new()
            .initial([event()
                .uid("old")
                .warning()
                .reason("BackOff")
                .regarding("Pod", Some("default"), "web-0", "pod-0")
                .count(4)
                .build()])
            .add(
                1,
                event()
                    .uid("new")
                    .warning()
                    .reason("FailedScheduling")
                    .message("0/1 nodes are available")
                    .regarding("Pod", Some("default"), "web-1", "pod-1")
                    .last_seen("2026-01-01T00:05:00Z")
                    .build(),
            ),
    );
    let mut app = App::start(cx, ports);
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let centre = state.services().notifications.clone();
    assert!(
        app.drawn("status-notifications"),
        "the bell is always there"
    );
    assert!(!app.drawn("status-notifications-count"));

    app.press("enter");
    settle(&mut app);
    assert!(centre.is_watching(&cluster));
    assert_eq!(
        events.live_feeds(),
        1,
        "the centre shares the cluster's one feed"
    );
    assert!(centre.list().is_empty(), "the backlog is not news");

    events.clock().advance(TICK);
    settle(&mut app);
    let rows = centre.list();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "Pod default/web-1: FailedScheduling");
    assert!(app.drawn("status-notifications-count"));
    let toasts = app.read(|ws, cx| ws.toast_layer().read(cx).visible());
    assert!(
        toasts
            .iter()
            .any(|t| t.message.as_ref() == "0/1 nodes are available"),
        "{toasts:?}"
    );

    // The key shows the centre in the window's right dock, with the row.
    app.press(toggle_key());
    settle(&mut app);
    let open = app.read(|ws, cx| {
        ws.panel::<NotificationPanel>().is_some()
            && ws
                .dock(DockPosition::Right, cx)
                .is_some_and(|dock| dock.is_open())
    });
    assert!(open);
    assert!(app.drawn("notification-row-0"));
}

#[gpui::test]
fn the_commands_are_on_the_bus_with_tool_stubs(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for (id, owner) in [
        (
            CommandId::NOTIFICATIONS_TOGGLE_PANEL,
            "oxikube_workspace::notifications",
        ),
        (
            CommandId::NOTIFICATIONS_OPEN_LINK,
            "oxikube_workspace::notifications",
        ),
        (
            CommandId::NOTIFICATIONS_MARK_READ,
            "oxikube_app::notifications",
        ),
        (
            CommandId::NOTIFICATIONS_MARK_ALL_READ,
            "oxikube_app::notifications",
        ),
        (
            CommandId::NOTIFICATIONS_MUTE_CLUSTER,
            "oxikube_app::notifications",
        ),
        (
            CommandId::NOTIFICATIONS_UNMUTE_CLUSTER,
            "oxikube_app::notifications",
        ),
    ] {
        assert_eq!(bus.owner(id), Some(owner), "{id}");
        assert!(bus.tool(id).is_some(), "{id} has an MCP tool stub");
    }
}

#[gpui::test]
fn the_toggle_command_opens_the_centre_and_a_mute_is_saved(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    let state_port = ports.state.clone();
    let mut app = App::start(cx, ports);
    super::notifications::dispatch(&mut app, Command::NotificationsTogglePanel);
    settle(&mut app);
    let open = app.read(|ws, cx| {
        ws.dock(DockPosition::Right, cx)
            .is_some_and(|dock| dock.is_open())
    });
    assert!(open, "the palette's command shows the panel");

    let cluster = TestPorts::cluster_id();
    super::notifications::dispatch(
        &mut app,
        Command::NotificationsMuteCluster {
            cluster: cluster.clone(),
        },
    );
    let stored = futures::executor::block_on(
        state_port.kv_get(&StateKey::new(oxikube_app::notifications::MUTED_KEY).unwrap()),
    )
    .unwrap()
    .expect("saved");
    assert_eq!(stored, serde_json::json!({ "clusters": [cluster] }));
}

fn warning(uid: &str, reason: &str, kind: &str, name: &str) -> oxikube_domain::event::Event {
    event()
        .uid(uid)
        .warning()
        .reason(reason)
        .message(&format!("{reason} {name}"))
        .regarding(kind, Some("default"), name, name)
        .last_seen("2026-01-01T00:05:00Z")
        .build()
}

#[gpui::test]
fn the_events_settings_decide_what_notifies_and_a_hot_reload_applies_at_once(
    cx: &mut TestAppContext,
) {
    let ports = TestPorts::seeded();
    let cluster = TestPorts::cluster_id();
    let events = ports.connector.ports_for(&cluster).events;
    events.install(
        FakeEventFeed::new()
            .add(1, warning("a", "FailedScheduling", "Pod", "web-1"))
            .add(2, warning("b", "BackOff", "Pod", "web-2"))
            .add(3, warning("c", "DeadlineExceeded", "Job", "batch"))
            .add(4, warning("d", "Unhealthy", "Pod", "web-3")),
    );
    let mut app = App::start_with(cx, ports, |cx| {
        cx.update_global::<oxikube_settings::SettingsStore, _>(|store, _| {
            store
                .set_user_settings(r#"{ "events": { "ignore_reasons": ["FailedScheduling"] } }"#)
                .unwrap()
        });
    });
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let centre = state.services().notifications.clone();
    assert_eq!(
        centre.rules().ignored_reasons.len(),
        1,
        "settings.json is read at start"
    );

    app.press("enter");
    settle(&mut app);
    assert!(centre.is_watching(&cluster));
    events.clock().advance(TICK);
    settle(&mut app);
    assert!(centre.list().is_empty(), "an ignored reason never notifies");

    // Edit settings.json: only Jobs now, and the old ignore-list is gone.
    super::notifications::edit_settings(
        &mut app,
        r#"{ "events": { "watched_kinds": ["Job"], "rate_limit": { "max_raised": 7 } } }"#,
    );
    let rules = centre.rules();
    assert_eq!(rules.watched_kinds.len(), 1);
    assert!(rules.ignored_reasons.is_empty());
    assert_eq!(rules.max_raised, 7);
    assert!(
        centre.is_watching(&cluster),
        "still one watch after the new kinds"
    );
    for _ in 0..3 {
        events.clock().advance(TICK);
        settle(&mut app);
    }
    let titles: Vec<_> = centre.list().iter().map(|n| n.title.clone()).collect();
    assert_eq!(
        titles,
        ["Job default/batch: DeadlineExceeded"],
        "Pods are no longer watched"
    );
    assert_eq!(events.live_feeds(), 1, "still the cluster's one feed");
}

#[gpui::test]
fn the_other_sources_reach_the_running_app_through_the_services_hooks(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    // What E15, E24 and E27 are handed: the app's one service as the hooks port.
    let hooks = state.services().notifications.sources();
    emit_update_available(&*hooks, "0.6.1");
    emit_portforward_drop(&*hooks, "fwd-1");
    emit_agent_permission(&*hooks, "s1", "r1");
    settle(&mut app);

    assert!(
        app.drawn("status-notifications-count"),
        "the bell counts them"
    );
    let messages: Vec<String> = app
        .read(|ws, cx| ws.toast_layer().read(cx).visible())
        .iter()
        .map(|t| t.message.to_string())
        .collect();
    assert_eq!(messages.len(), 3, "{messages:?}");
    assert_eq!(state.services().notifications.unread(), 3);

    // The permission toast waits for the answer; the others go by themselves.
    app.vcx.executor().advance_clock(Duration::from_secs(120));
    app.vcx.run_until_parked();
    assert_eq!(
        app.read(|ws, cx| ws.toast_layer().read(cx).visible()).len(),
        1
    );
    hooks.agent_permission_resolved("s1", "r1");
    settle(&mut app);
    assert!(
        app.read(|ws, cx| ws.toast_layer().read(cx).visible())
            .is_empty()
    );
    assert_eq!(state.services().notifications.unread(), 2);

    // The panel lists all three, newest first.
    super::notifications::dispatch(&mut app, Command::NotificationsTogglePanel);
    settle(&mut app);
    assert!(app.drawn("notification-row-2"));
    assert!(!app.drawn("notification-row-3"));
}

#[gpui::test]
fn only_https_links_open_from_a_notification(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let ok = super::notifications::dispatch(
        &mut app,
        Command::NotificationsOpenLink {
            url: "https://example.com/release".to_owned(),
        },
    );
    assert!(ok.message.is_none());
    assert_eq!(
        app.vcx.opened_url().as_deref(),
        Some("https://example.com/release"),
        "the window's serve loop opened it"
    );

    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let refused = futures::executor::block_on(bus.dispatch(
        Command::NotificationsOpenLink {
            url: "file:///etc/passwd".to_owned(),
        },
        oxikube_app::command_bus::DispatchContext::new(oxikube_domain::Initiator::Ui, "me"),
    ));
    assert!(refused.is_err(), "{refused:?}");
    app.vcx.run_until_parked();
    assert_eq!(
        app.vcx.opened_url().as_deref(),
        Some("https://example.com/release"),
        "the refused link reached nothing"
    );
}
