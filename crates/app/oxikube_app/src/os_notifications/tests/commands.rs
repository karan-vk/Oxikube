//! The two bus commands, dispatched through a real `CommandBus`.

use std::sync::Arc;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use oxikube_domain::OxiResult;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_ports::ClusterPrefsTable;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeNotifierPort, FakeStatePort,
};
use parking_lot::Mutex;

use crate::command_bus::{CommandBus, CommandOutput, CommandRegistry, Outcome};
use crate::guard::MutationGuard;
use crate::os_notifications::{OsNotificationService, OsNotificationSettings, register_commands};
use crate::session::ClusterSessionManager;
use crate::testing::{cluster_context, ctx};

#[derive(Default)]
struct Writer {
    written: Mutex<Vec<bool>>,
}

impl OsNotificationSettings for Writer {
    fn set_enabled(&self, enabled: bool) -> BoxFuture<'static, OxiResult<()>> {
        self.written.lock().push(enabled);
        async { Ok(()) }.boxed()
    }
}

struct Fixture {
    bus: CommandBus,
    service: OsNotificationService,
    notifier: Arc<FakeNotifierPort>,
    writer: Arc<Writer>,
    sessions: ClusterSessionManager,
}

fn fixture() -> Fixture {
    let notifier = Arc::new(FakeNotifierPort::new());
    let service = OsNotificationService::new(notifier.clone());
    let writer = Arc::new(Writer::default());
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::os_notifications", |r| {
            register_commands(r, service.clone(), writer.clone())
        })
        .expect("the commands register");
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new().with_contexts([cluster_context("a")])),
        clock.clone(),
    );
    sessions.set_prefs_table(ClusterPrefsTable::default());
    let guard = MutationGuard::new(sessions.clone(), Arc::new(FakeStatePort::new()), clock);
    Fixture {
        bus: CommandBus::new(registry, guard),
        service,
        notifier,
        writer,
        sessions,
    }
}

async fn run(f: &Fixture, command: Command) -> CommandOutput {
    match f.bus.dispatch(command, ctx(Initiator::Ui)).await {
        Ok(Outcome::Completed(output)) => output,
        other => panic!("expected a completed command, got {other:?}"),
    }
}

#[test]
fn both_commands_have_tool_stubs_an_agent_may_call() {
    let f = fixture();
    for (command, tool) in [
        (
            CommandId::APP_TOGGLE_OS_NOTIFICATIONS,
            "app.app_toggle_os_notifications",
        ),
        (
            CommandId::APP_SEND_TEST_NOTIFICATION,
            "app.app_send_test_notification",
        ),
    ] {
        let stub = f.bus.tool(command).expect("a tool stub");
        assert_eq!(stub.name.as_str(), tool);
        assert!(f.bus.agent_tools(false).any(|t| t.name.as_str() == tool));
    }
}

#[tokio::test]
async fn the_toggle_writes_the_setting_and_turns_the_notifier_on_and_off() {
    let f = fixture();
    let on = run(&f, Command::AppToggleOsNotifications { enabled: None }).await;
    assert_eq!(on.data.as_ref().unwrap()["enabled"], true);
    assert_eq!(on.data.unwrap()["status"], "ready");
    assert!(f.service.wanted());
    assert!(
        f.notifier.is_enabled(),
        "the user's opt-in reached the port"
    );

    let off = run(&f, Command::AppToggleOsNotifications { enabled: None }).await;
    assert_eq!(off.data.unwrap()["enabled"], false);
    assert!(!f.notifier.is_enabled());

    run(
        &f,
        Command::AppToggleOsNotifications {
            enabled: Some(false),
        },
    )
    .await;
    assert_eq!(*f.writer.written.lock(), [true, false, false]);
}

#[tokio::test]
async fn the_test_notification_needs_the_opt_in() {
    let f = fixture();
    let off = run(&f, Command::AppSendTestNotification).await;
    assert_eq!(off.data.unwrap()["sent"], false);
    assert!(off.message.unwrap().contains("off"));
    assert!(f.notifier.recorded_calls().is_empty());

    run(
        &f,
        Command::AppToggleOsNotifications {
            enabled: Some(true),
        },
    )
    .await;
    let sent = run(&f, Command::AppSendTestNotification).await;
    assert_eq!(sent.data.unwrap()["sent"], true);
    assert_eq!(f.notifier.shown().len(), 1);
}

#[tokio::test]
async fn a_missing_daemon_is_a_message_not_an_error() {
    let f = fixture();
    f.notifier
        .script()
        .set_enabled
        .push_ok(oxikube_ports::NotifierStatus::Unavailable(
            "no daemon".into(),
        ));
    let output = run(
        &f,
        Command::AppToggleOsNotifications {
            enabled: Some(true),
        },
    )
    .await;
    assert_eq!(output.data.as_ref().unwrap()["status"], "unavailable");
    assert!(output.message.unwrap().contains("no daemon"));
    let test = run(&f, Command::AppSendTestNotification).await;
    assert_eq!(test.data.unwrap()["sent"], false);
}

#[tokio::test]
async fn both_commands_run_on_a_read_only_cluster() {
    let f = fixture();
    f.sessions.open_configured(&cluster_context("a"));
    f.sessions
        .set_prefs_table(ClusterPrefsTable::new(oxikube_ports::ClusterPrefs {
            read_only: true,
            ..Default::default()
        }));
    assert!(
        f.sessions
            .get(&crate::testing::id("a"))
            .unwrap()
            .read_only()
    );
    run(
        &f,
        Command::AppToggleOsNotifications {
            enabled: Some(true),
        },
    )
    .await;
    run(&f, Command::AppSendTestNotification).await;
    assert_eq!(f.notifier.shown().len(), 1);
}
