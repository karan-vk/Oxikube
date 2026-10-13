//! OS notifications in the running app (E14-S06): the opt-in in `settings.json`, the two commands
//! on the bus with their tool stubs, and a click on a notification bringing the window to the front
//! and opening the object, all over the fake notifier of `TestPorts`.

use std::cell::RefCell;
use std::rc::Rc;

use futures::executor::block_on;
use gpui::{BorrowAppContext as _, TestAppContext};
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_ports::{Notification, NotificationLevel};
use oxikube_resources_ui::detail::DetailDrawer;
use oxikube_settings::SettingsStore;
use oxikube_testkit::{NotifierCall, TestPorts};

use super::App;
use crate::app_state::AppState;

/// Runs `command` on the app's bus and settles the app. The handler may wait for the foreground
/// (the settings writer applies its edit there), so it runs as a foreground task, never under a
/// blocking `block_on`.
fn dispatch(app: &mut App, command: Command) -> oxikube_app::command_bus::CommandOutput {
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let slot = Rc::new(RefCell::new(None));
    let result = slot.clone();
    app.vcx.update(|_, cx| {
        cx.spawn(async move |_| {
            let outcome = bus
                .dispatch(command, DispatchContext::new(Initiator::Ui, "me"))
                .await;
            *result.borrow_mut() = Some(outcome);
        })
        .detach();
    });
    app.vcx.run_until_parked();
    let outcome = slot.borrow_mut().take().expect("the command finished");
    match outcome.expect("the command runs") {
        oxikube_app::command_bus::Outcome::Completed(output) => output,
        other => panic!("expected a completed command, got {other:?}"),
    }
}

fn edit_settings(app: &mut App, text: &str) {
    let text = text.to_owned();
    app.vcx.update(|_, cx| {
        cx.update_global::<SettingsStore, _>(|store, _| store.set_user_settings(&text).unwrap());
    });
    app.vcx.run_until_parked();
}

fn set_enabled_calls(app: &App) -> Vec<bool> {
    app.ports
        .notifier
        .recorded_calls()
        .into_iter()
        .filter_map(|call| match call {
            NotifierCall::SetEnabled(on) => Some(on),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn the_commands_are_on_the_bus_with_tool_stubs(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for (id, tool) in [
        (
            CommandId::APP_TOGGLE_OS_NOTIFICATIONS,
            "app.app_toggle_os_notifications",
        ),
        (
            CommandId::APP_SEND_TEST_NOTIFICATION,
            "app.app_send_test_notification",
        ),
    ] {
        assert_eq!(bus.owner(id), Some("oxikube_app::os_notifications"), "{id}");
        assert_eq!(bus.tool(id).map(|t| t.name.as_str()), Some(tool), "{id}");
    }
}

#[gpui::test]
fn nothing_contacts_the_os_while_the_opt_in_is_off(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.press("enter");
    assert!(
        app.ports.notifier.recorded_calls().is_empty(),
        "start-up and connecting a cluster never touch the notifier"
    );
    let output = dispatch(&mut app, Command::AppSendTestNotification);
    assert_eq!(output.data.unwrap()["sent"], false);
    assert!(output.message.unwrap().contains("off"));
    assert!(app.ports.notifier.recorded_calls().is_empty());
}

#[gpui::test]
fn the_toggle_command_is_the_opt_in_and_the_setting_follows_it(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let on = dispatch(
        &mut app,
        Command::AppToggleOsNotifications { enabled: None },
    );
    assert_eq!(on.data.unwrap()["status"], "ready");
    assert_eq!(set_enabled_calls(&app), [true], "the OS was asked once");
    assert!(app.ports.notifier.is_enabled());

    let sent = dispatch(&mut app, Command::AppSendTestNotification);
    assert_eq!(sent.data.unwrap()["sent"], true);
    assert_eq!(app.ports.notifier.shown().len(), 1);

    // The setting is what the toggle wrote.
    let on_in_settings = app
        .vcx
        .update(|_, cx| oxikube_events_ui::EventsSettings::current(cx).os_notifications);
    assert!(on_in_settings);

    // Editing settings.json by hand turns it off again, without a restart.
    edit_settings(&mut app, r#"{ "events": { "os_notifications": false } }"#);
    assert_eq!(set_enabled_calls(&app), [true, false]);
    assert!(!app.ports.notifier.is_enabled());
}

#[gpui::test]
fn an_opt_in_already_in_settings_json_is_applied_at_start(cx: &mut TestAppContext) {
    let app = App::start_with(cx, TestPorts::seeded(), |cx| {
        cx.update_global::<SettingsStore, _>(|store, _| {
            store
                .set_user_settings(r#"{ "events": { "os_notifications": true } }"#)
                .unwrap()
        });
    });
    assert!(app.ports.notifier.is_enabled());
}

#[gpui::test]
fn clicking_a_notification_opens_the_object_in_the_window(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.serve([super::resources::kind("", "v1", "Pod", "pods")]);
    app.ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .resources
        .insert(oxikube_testkit::fixtures::pod_running());
    app.press("enter");
    app.tick();
    app.click("sidebar-entry-workloads/pods");
    app.tick();
    assert_eq!(app.tables(), ["Pod"]);
    dispatch(
        &mut app,
        Command::AppToggleOsNotifications {
            enabled: Some(true),
        },
    );

    // A Warning about the seeded pod, as the notification centre will hand it over (E14-S05).
    let pod = ResourceRef::namespaced(
        TestPorts::cluster_id(),
        Gvk::new("", "v1", "Pod"),
        "demo",
        "web-running",
    );
    let warning = Notification::new(
        "Pod web-running",
        "Back-off restarting",
        NotificationLevel::Warning,
    )
    .with_tag("warning/pod")
    .with_action_target(pod);
    let service = app
        .vcx
        .update(|_, cx| AppState::global(cx).services().os_notifications.clone());
    assert_eq!(
        block_on(service.notify(&warning)),
        oxikube_app::Delivery::Sent
    );
    assert!(!app.drawn("detail-view"), "nothing is open yet");

    // The user clicks it: the fake plays the adapter thread.
    app.ports
        .notifier
        .click_last()
        .expect("a shown notification");
    app.tick();
    assert!(
        app.drawn("detail-view"),
        "the click opened the pod's detail"
    );
    let ws = app.tab_workspace();
    let shown = app.vcx.update(|_, cx| {
        ws.read(cx)
            .panel::<DetailDrawer>()
            .and_then(|drawer| drawer.read(cx).view().cloned())
    });
    assert!(shown.is_some(), "in the cluster tab's drawer");
}
