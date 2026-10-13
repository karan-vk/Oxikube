use std::sync::Arc;

use futures::executor::block_on;
use oxikube_domain::OxiError;
use oxikube_ports::{NotifierStatus, unavailable};
use oxikube_testkit::NotifierCall;
use parking_lot::Mutex;

use super::{service, target, warning};
use crate::os_notifications::{ClickAction, Delivery};

#[test]
fn nothing_reaches_the_port_until_the_user_opts_in() {
    let (service, notifier) = service();
    assert!(!service.wanted());
    assert_eq!(block_on(service.notify(&warning("boom"))), Delivery::Off);
    assert!(
        notifier.recorded_calls().is_empty(),
        "the port was not touched"
    );

    assert_eq!(block_on(service.apply(true)), NotifierStatus::Ready);
    assert_eq!(block_on(service.notify(&warning("boom"))), Delivery::Sent);
    assert_eq!(notifier.shown().len(), 1);

    assert_eq!(block_on(service.apply(false)), NotifierStatus::Off);
    assert_eq!(block_on(service.notify(&warning("boom"))), Delivery::Off);
    assert!(notifier.shown().is_empty());
}

#[test]
fn applying_the_same_state_twice_asks_the_os_once() {
    let (service, notifier) = service();
    block_on(service.apply(true));
    block_on(service.apply(true));
    block_on(service.apply(false));
    block_on(service.apply(false));
    let switches: Vec<_> = notifier
        .recorded_calls()
        .into_iter()
        .filter(|call| matches!(call, NotifierCall::SetEnabled(_)))
        .collect();
    assert_eq!(
        switches,
        vec![
            NotifierCall::SetEnabled(true),
            NotifierCall::SetEnabled(false)
        ]
    );
}

#[test]
fn text_is_redacted_and_capped_before_the_port_sees_it() {
    let (service, notifier) = service();
    block_on(service.apply(true));
    let secret = warning(&format!(
        "token=abcdef1234567890abcdef0123456789 {}",
        "x".repeat(500)
    ));
    block_on(service.notify(&secret));
    let shown = notifier.shown();
    assert!(!shown[0].body.contains("abcdef1234567890abcdef0123456789"));
    assert!(shown[0].body.chars().count() <= oxikube_ports::MAX_BODY_CHARS);
}

#[test]
fn a_system_without_a_daemon_is_explained_once_and_never_panics() {
    let (service, notifier) = service();
    notifier
        .script()
        .set_enabled
        .push_ok(NotifierStatus::Unavailable("no D-Bus session".into()));
    assert_eq!(
        block_on(service.apply(true)),
        NotifierStatus::Unavailable("no D-Bus session".into())
    );
    let Delivery::Unavailable { hint: Some(hint) } = block_on(service.notify(&warning("a"))) else {
        panic!("the first call carries the hint");
    };
    assert!(hint.contains("no D-Bus session"), "{hint}");
    assert_eq!(
        block_on(service.notify(&warning("b"))),
        Delivery::Unavailable { hint: None }
    );
    assert!(
        notifier.notifications().is_empty(),
        "nothing was sent to a dead service"
    );

    // Starting a daemon and switching the setting off and on again retries.
    block_on(service.apply(false));
    assert_eq!(block_on(service.apply(true)), NotifierStatus::Ready);
    assert_eq!(block_on(service.notify(&warning("c"))), Delivery::Sent);
}

#[test]
fn a_service_that_dies_later_flips_to_unavailable() {
    let (service, notifier) = service();
    block_on(service.apply(true));
    notifier.script().notify.push_err(unavailable("bus closed"));
    let Delivery::Unavailable { hint } = block_on(service.notify(&warning("a"))) else {
        panic!("unavailable");
    };
    assert!(hint.is_some());
    assert!(matches!(service.status(), NotifierStatus::Unavailable(_)));
}

#[test]
fn an_error_while_enabling_keeps_the_opt_in_and_reports_unavailable() {
    let (service, notifier) = service();
    notifier
        .script()
        .set_enabled
        .push_err(OxiError::internal("boom"));
    assert!(matches!(
        block_on(service.apply(true)),
        NotifierStatus::Unavailable(_)
    ));
    assert!(service.wanted(), "the user's choice stands");
}

#[test]
fn another_failure_is_reported_not_raised() {
    let (service, notifier) = service();
    block_on(service.apply(true));
    notifier
        .script()
        .notify
        .push_err(OxiError::internal("refused"));
    assert_eq!(
        block_on(service.notify(&warning("a"))),
        Delivery::Failed("refused".into())
    );
    assert_eq!(service.status(), NotifierStatus::Ready);
}

#[test]
fn a_click_reaches_the_sink_with_the_cluster_and_the_object() {
    let (service, notifier) = service();
    let clicks = Arc::new(Mutex::new(Vec::new()));
    let sink = clicks.clone();
    service.on_click(Arc::new(move |action| sink.lock().push(action)));
    block_on(service.apply(true));

    let event = warning("Back-off").with_action_target(target());
    block_on(service.notify(&event));
    notifier.click_last().expect("a notification to click");
    assert_eq!(
        *clicks.lock(),
        vec![ClickAction {
            cluster: Some(target().cluster),
            open: Some(target()),
        }]
    );
}

#[test]
fn the_test_notification_replaces_itself_and_clicks_to_nothing() {
    let (service, notifier) = service();
    assert_eq!(block_on(service.send_test()), Delivery::Off);
    block_on(service.apply(true));
    assert_eq!(block_on(service.send_test()), Delivery::Sent);
    assert_eq!(block_on(service.send_test()), Delivery::Sent);
    assert_eq!(notifier.shown().len(), 1, "same tag: replaced, not stacked");
    let activation = notifier.shown()[0].activation();
    assert_eq!(ClickAction::from(activation).commands(), vec![]);
}

/// A port whose `set_enabled(true)` answers only when the test says so, like an adapter that is
/// still connecting to the OS.
struct SlowPort {
    answer: Mutex<Option<futures::channel::oneshot::Receiver<NotifierStatus>>>,
    switches: std::sync::atomic::AtomicUsize,
    notified: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl oxikube_ports::NotifierPort for SlowPort {
    async fn set_enabled(&self, enabled: bool) -> oxikube_domain::OxiResult<NotifierStatus> {
        self.switches
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let answer = self.answer.lock().take();
        match (enabled, answer) {
            (true, Some(answer)) => Ok(answer.await.unwrap_or(NotifierStatus::Ready)),
            (true, None) => Ok(NotifierStatus::Ready),
            (false, _) => Ok(NotifierStatus::Off),
        }
    }

    async fn notify(&self, _: &oxikube_ports::Notification) -> oxikube_domain::OxiResult<()> {
        self.notified
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    async fn clear(&self, _: &str) -> oxikube_domain::OxiResult<()> {
        Ok(())
    }

    fn on_activated(&self, _: oxikube_ports::ActivationHandler) {}
}

#[test]
fn an_overlapping_apply_returns_the_status_the_first_arrived_at() {
    let (answer, answered) = futures::channel::oneshot::channel();
    let port = Arc::new(SlowPort {
        answer: Mutex::new(Some(answered)),
        switches: Default::default(),
        notified: Default::default(),
    });
    let service = crate::os_notifications::OsNotificationService::new(port.clone());
    let unavailable = NotifierStatus::Unavailable("no daemon".into());

    // The settings observer and the toggle command both apply `true` for one change.
    let observer = service.apply(true);
    let command = service.apply(true);
    let meanwhile = async {
        // The port is still connecting: nothing is sent to it, and nothing claims it was.
        assert_eq!(service.notify(&warning("early")).await, Delivery::Off);
        answer.send(unavailable.clone()).unwrap();
    };
    let (first, second, ()) = block_on(async { futures::join!(observer, command, meanwhile) });

    assert_eq!(first, unavailable);
    assert_eq!(second, unavailable, "not the half-done `Off`");
    assert_eq!(port.switches.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(port.notified.load(std::sync::atomic::Ordering::SeqCst), 0);
    // The user is told about the missing daemon, once.
    assert!(matches!(
        block_on(service.notify(&warning("later"))),
        Delivery::Unavailable { hint: Some(_) }
    ));
}
