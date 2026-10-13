//! The real platform backend, with no cluster and no window (E14-S06).
//!
//! Runs everywhere, including a CI machine with no notification daemon and no D-Bus session: the
//! opt-in must answer with a status, never panic, and a notifier that is off must not contact the
//! OS at all. It shows nothing on the desktop: a notification is only sent when
//! `OXIKUBE_TEST_OS_NOTIFY=1` is set (the manual checklist in the PR: run it on macOS and on a
//! Linux desktop and look for the notification).

use futures::executor::block_on;
use oxikube_notify_os::OsNotifier;
use oxikube_ports::{
    Notification, NotificationLevel, NotifierPort, NotifierStatus, is_unavailable,
};

#[test]
fn off_by_default_and_silent() {
    let notifier = OsNotifier::new();
    assert_eq!(notifier.status(), NotifierStatus::Off);
    let note = Notification::new("Oxikube", "never shown", NotificationLevel::Info);
    block_on(notifier.notify(&note)).expect("an off notifier accepts and drops it");
    assert_eq!(notifier.status(), NotifierStatus::Off);
}

#[test]
fn turning_it_on_answers_with_a_status_whatever_the_machine_has() {
    let notifier = OsNotifier::new();
    let status = block_on(notifier.set_enabled(true)).expect("a status, not an error");
    assert!(
        matches!(
            status,
            NotifierStatus::Ready | NotifierStatus::Unavailable(_)
        ),
        "{status:?}"
    );
    if let NotifierStatus::Unavailable(reason) = &status {
        // A machine with no daemon: notify reports it as such and does not panic.
        assert!(!reason.is_empty());
        let note = Notification::new("Oxikube", "no daemon", NotificationLevel::Warning);
        let error = block_on(notifier.notify(&note)).unwrap_err();
        assert!(is_unavailable(&error), "{error}");
    }
    assert_eq!(
        block_on(notifier.set_enabled(false)).unwrap(),
        NotifierStatus::Off
    );
}

/// Sends one real notification. Manual: `OXIKUBE_TEST_OS_NOTIFY=1 cargo test -p oxikube_notify_os
/// --test degrades_gracefully -- --ignored`.
#[test]
#[ignore = "shows a notification on the desktop"]
fn shows_one_notification_when_asked() {
    if std::env::var_os("OXIKUBE_TEST_OS_NOTIFY").is_none() {
        return;
    }
    let notifier = OsNotifier::new();
    let status = block_on(notifier.set_enabled(true)).unwrap();
    assert!(!matches!(status, NotifierStatus::Off), "{status}");
    let note = Notification::new(
        "Oxikube test notification",
        "E14-S06 manual check: this should appear in the notification centre.",
        NotificationLevel::Info,
    )
    .with_tag("oxikube/manual-test");
    block_on(notifier.notify(&note)).expect("the notification is handed to the OS");
    // Give the delivery thread a moment before the test process exits.
    std::thread::sleep(std::time::Duration::from_secs(3));
}
