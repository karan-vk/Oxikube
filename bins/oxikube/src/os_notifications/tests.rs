//! The setting reaches the service, and the writer reaches the setting.

use std::sync::Arc;

use futures::FutureExt as _;
use gpui::{BorrowAppContext as _, TestAppContext};
use oxikube_app::OsNotificationService;
use oxikube_app::os_notifications::OsNotificationSettings as _;
use oxikube_events_ui::EventsSettings;
use oxikube_settings::SettingsStore;
use oxikube_testkit::{FakeNotifierPort, NotifierCall};

use super::{SettingsOsNotificationWriter, follow_setting};
use crate::app_state::AppState;

fn edit_settings_file(cx: &mut TestAppContext, text: &str) {
    let text = text.to_owned();
    cx.update(|cx| {
        cx.update_global::<SettingsStore, _>(|store, _| store.set_user_settings(&text).unwrap());
    });
    cx.run_until_parked();
}

fn switches(notifier: &FakeNotifierPort) -> Vec<bool> {
    notifier
        .recorded_calls()
        .into_iter()
        .filter_map(|call| match call {
            NotifierCall::SetEnabled(on) => Some(on),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn the_service_follows_events_os_notifications_live(cx: &mut TestAppContext) {
    let notifier = Arc::new(FakeNotifierPort::new());
    let service = cx.update(|cx| {
        AppState::test(cx);
        let service = OsNotificationService::new(notifier.clone());
        follow_setting(&service, cx).detach();
        service
    });
    cx.run_until_parked();
    assert!(!service.wanted());
    assert!(
        notifier.recorded_calls().is_empty(),
        "off at start: the notifier is not even told"
    );

    edit_settings_file(cx, r#"{ "events": { "os_notifications": true } }"#);
    assert!(service.wanted());
    assert_eq!(switches(&notifier), [true]);

    // An unrelated edit does not wake it.
    edit_settings_file(
        cx,
        r#"{ "events": { "os_notifications": true }, "ui_scale": 1.1 }"#,
    );
    assert_eq!(switches(&notifier), [true]);

    edit_settings_file(cx, "{}");
    assert!(!service.wanted());
    assert_eq!(switches(&notifier), [true, false]);
}

#[gpui::test]
fn the_writer_edits_the_setting_and_keeps_the_rest_of_the_file(cx: &mut TestAppContext) {
    cx.update(|cx| {
        AppState::test(cx);
    });
    edit_settings_file(cx, "{\n  // mine\n  \"ui_scale\": 1.2\n}\n");
    let writer = cx.update(SettingsOsNotificationWriter::new);
    let done = writer.set_enabled(true);
    cx.run_until_parked();
    done.now_or_never().expect("the edit is done").unwrap();
    cx.read(|cx| assert!(EventsSettings::current(cx).os_notifications));
    let text = cx.read(|cx| {
        cx.global::<SettingsStore>()
            .user_settings_text()
            .unwrap()
            .to_owned()
    });
    assert!(
        text.contains("// mine") && text.contains("\"ui_scale\": 1.2"),
        "{text}"
    );
    assert!(text.contains("\"os_notifications\": true"), "{text}");
}
