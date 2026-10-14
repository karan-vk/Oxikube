//! The four bus commands, dispatched through a real `CommandBus`, with their tool stubs.

use std::sync::Arc;

use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_ports::ClusterPrefsTable;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};

use super::{cluster, input, t0};
use crate::command_bus::{CommandBus, CommandOutput, CommandRegistry, Outcome};
use crate::events::{EventService, EventsConfig};
use crate::guard::MutationGuard;
use crate::notifications::{COMMANDS, NotificationService, register_commands};
use crate::session::ClusterSessionManager;
use crate::testing::{cluster_context, ctx};

struct Fixture {
    bus: CommandBus,
    centre: NotificationService,
}

fn fixture() -> Fixture {
    let clock = Arc::new(FakeClockPort::new(t0()));
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new().with_contexts([cluster_context("a")])),
        clock.clone(),
    );
    sessions.set_prefs_table(ClusterPrefsTable::default());
    let events = EventService::new(sessions.clone(), clock.clone(), EventsConfig::default());
    let state = Arc::new(FakeStatePort::new());
    let centre = NotificationService::new(events, sessions.clone(), state.clone(), clock.clone());
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::notifications", |r| {
            register_commands(r, centre.clone())
        })
        .expect("the commands register");
    let guard = MutationGuard::new(sessions, state, clock);
    Fixture {
        bus: CommandBus::new(registry, guard),
        centre,
    }
}

async fn run(f: &Fixture, command: Command) -> CommandOutput {
    match f.bus.dispatch(command, ctx(Initiator::Ui)).await {
        Ok(Outcome::Completed(output)) => output,
        other => panic!("expected a completed command, got {other:?}"),
    }
}

#[test]
fn every_command_has_a_tool_stub_an_agent_may_call() {
    let f = fixture();
    for id in COMMANDS {
        let stub = f.bus.tool(id).expect("a tool stub");
        assert!(
            f.bus.agent_tools(false).any(|t| t.name == stub.name),
            "{id} is offered to agents"
        );
    }
}

#[tokio::test]
async fn mark_read_and_mark_all_read() {
    let f = fixture();
    let a = f.centre.submit(input(&cluster(), "a")).unwrap().id;
    f.centre.submit(input(&cluster(), "b"));
    let out = run(&f, Command::NotificationsMarkRead { id: a.0 }).await;
    assert_eq!(out.data.unwrap()["unread"], 1);
    assert!(f.centre.get(a).unwrap().read);
    let out = run(&f, Command::NotificationsMarkAllRead).await;
    assert_eq!(out.data.unwrap()["marked"], 1);
    assert_eq!(f.centre.unread(), 0);
    // An id that is not listed is an error an agent can read.
    assert!(
        f.bus
            .dispatch(
                Command::NotificationsMarkRead { id: 999 },
                ctx(Initiator::Ui)
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn mute_and_unmute_a_cluster() {
    let f = fixture();
    let out = run(&f, Command::NotificationsMuteCluster { cluster: cluster() }).await;
    assert_eq!(out.data.unwrap()["muted"], true);
    assert!(f.centre.is_muted(&cluster()));
    run(
        &f,
        Command::NotificationsUnmuteCluster { cluster: cluster() },
    )
    .await;
    assert!(!f.centre.is_muted(&cluster()));
}
