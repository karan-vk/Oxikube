//! The cluster commands the catalog dispatches.

use futures::FutureExt as _;
use oxikube_domain::ErrorKind;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::session::{ClusterSessionState, SessionPhase};
use oxikube_testkit::ConnectorCall;

use super::{Harness, id};
use crate::catalog::{ClusterCommandOutcome, ClusterCommands};
use oxikube_domain::OxiResult;

fn commands(h: &Harness) -> ClusterCommands {
    ClusterCommands::new(h.sessions.clone(), h.catalog.clone())
}

fn run(h: &Harness, command: Command) -> OxiResult<ClusterCommandOutcome> {
    commands(h)
        .handle(&command)
        .now_or_never()
        .expect("fakes do not wait")
}

#[test]
fn connect_connects_the_cluster_and_records_that_it_was_used() {
    let h = Harness::new();
    h.clock.advance(std::time::Duration::from_secs(5));
    let outcome = run(&h, Command::ClusterConnect { cluster: id("b") }).expect("connect");
    assert_eq!(
        outcome,
        ClusterCommandOutcome::Connected(ClusterSessionState::Ready)
    );
    assert_eq!(
        h.sessions.get(&id("b")).unwrap().phase(),
        SessionPhase::Ready
    );
    assert_eq!(h.entry("b").last_used, Some(h.now()));
    assert_eq!(h.entry("a").last_used, None);
    assert!(
        h.connector
            .recorded_calls()
            .iter()
            .any(|c| matches!(c, ConnectorCall::Connect { .. })),
        "{:?}",
        h.connector.recorded_calls()
    );
}

#[test]
fn a_failed_connection_is_an_outcome_not_an_error_and_still_counts_as_used() {
    let h = Harness::new();
    h.connector
        .connect_script_for(&id("a"))
        .push_err(oxikube_domain::OxiError::auth("token expired", false));
    let outcome = run(&h, Command::ClusterConnect { cluster: id("a") }).expect("connect");
    assert!(
        matches!(
            outcome,
            ClusterCommandOutcome::Connected(ClusterSessionState::AuthRequired { .. })
        ),
        "{outcome:?}"
    );
    assert!(h.entry("a").last_used.is_some());
}

#[test]
fn connect_survives_a_state_db_that_cannot_be_written() {
    let h = Harness::new();
    h.state
        .script()
        .table_put
        .push_err(oxikube_domain::OxiError::internal("disk full"));
    let outcome = run(&h, Command::ClusterConnect { cluster: id("a") }).expect("connect");
    assert_eq!(
        outcome,
        ClusterCommandOutcome::Connected(ClusterSessionState::Ready)
    );
}

#[test]
fn connecting_a_cluster_that_is_not_in_the_catalog_is_not_found() {
    let h = Harness::new();
    let ghost = ClusterId::new("/nowhere", &ContextName::new("ghost"));
    let error =
        run(&h, Command::ClusterConnect { cluster: ghost }).expect_err("not in the catalog");
    assert_eq!(error.kind(), ErrorKind::NotFound);
}

#[test]
fn disconnect_closes_a_connected_cluster_and_ignores_one_never_opened() {
    let h = Harness::new();
    run(&h, Command::ClusterConnect { cluster: id("a") }).expect("connect");
    let outcome = run(&h, Command::ClusterDisconnect { cluster: id("a") }).expect("disconnect");
    assert_eq!(outcome, ClusterCommandOutcome::Disconnected);
    assert_eq!(
        h.sessions.get(&id("a")).unwrap().phase(),
        SessionPhase::Disconnected
    );
    let outcome = run(&h, Command::ClusterDisconnect { cluster: id("c") }).expect("no-op");
    assert_eq!(outcome, ClusterCommandOutcome::Disconnected);
}

#[test]
fn reconnect_retries_a_cluster_that_needed_auth() {
    let h = Harness::new();
    h.connector
        .connect_script_for(&id("a"))
        .push_err(oxikube_domain::OxiError::auth("token expired", false));
    let first = run(&h, Command::ClusterConnect { cluster: id("a") }).expect("connect");
    assert!(
        matches!(
            first,
            ClusterCommandOutcome::Connected(ClusterSessionState::AuthRequired { .. })
        ),
        "{first:?}"
    );
    let retry = run(&h, Command::ClusterReconnect { cluster: id("a") }).expect("reconnect");
    assert_eq!(
        retry,
        ClusterCommandOutcome::Connected(ClusterSessionState::Ready)
    );
    assert_eq!(
        h.sessions.get(&id("a")).unwrap().phase(),
        SessionPhase::Ready
    );
}

#[test]
fn reconnect_replaces_a_live_connection() {
    let h = Harness::new();
    run(&h, Command::ClusterConnect { cluster: id("a") }).expect("connect");
    let retry = run(&h, Command::ClusterReconnect { cluster: id("a") }).expect("reconnect");
    assert_eq!(
        retry,
        ClusterCommandOutcome::Connected(ClusterSessionState::Ready)
    );
    assert_eq!(h.connector.live_connections(&id("a")), 1);
}

#[test]
fn cancel_connect_stops_only_an_attempt_in_flight() {
    let h = Harness::new();
    h.connector.hold();
    let handler = commands(&h);
    let command = Command::ClusterConnect { cluster: id("a") };
    let mut connect = Box::pin(async { handler.handle(&command).await });
    assert!(connect.as_mut().now_or_never().is_none());
    assert_eq!(
        h.sessions.get(&id("a")).unwrap().phase(),
        SessionPhase::Connecting
    );

    let outcome = run(&h, Command::ClusterCancelConnect { cluster: id("a") }).expect("cancel");
    assert_eq!(outcome, ClusterCommandOutcome::Cancelled(true));
    assert_eq!(
        h.sessions.get(&id("a")).unwrap().phase(),
        SessionPhase::Disconnected
    );
    // The attempt's own future ends with the state it was cancelled into.
    assert_eq!(
        connect.now_or_never().expect("aborted").expect("connect"),
        ClusterCommandOutcome::Connected(ClusterSessionState::Disconnected)
    );
    assert_eq!(h.connector.cancelled(), 1);

    // Nothing in flight any more (and one never opened): nothing to cancel, nothing changes.
    let again = run(&h, Command::ClusterCancelConnect { cluster: id("a") }).expect("cancel");
    assert_eq!(again, ClusterCommandOutcome::Cancelled(false));
    let ghost = run(&h, Command::ClusterCancelConnect { cluster: id("c") }).expect("cancel");
    assert_eq!(ghost, ClusterCommandOutcome::Cancelled(false));
}

#[test]
fn cancel_connect_leaves_a_connected_cluster_alone() {
    let h = Harness::new();
    run(&h, Command::ClusterConnect { cluster: id("a") }).expect("connect");
    let outcome = run(&h, Command::ClusterCancelConnect { cluster: id("a") }).expect("cancel");
    assert_eq!(outcome, ClusterCommandOutcome::Cancelled(false));
    assert_eq!(
        h.sessions.get(&id("a")).unwrap().phase(),
        SessionPhase::Ready
    );
}

#[test]
fn the_session_knows_the_api_server_of_its_catalog_entry() {
    let h = Harness::new();
    run(&h, Command::ClusterConnect { cluster: id("b") }).expect("connect");
    assert_eq!(
        h.sessions.get(&id("b")).unwrap().server(),
        Some("https://b.example:6443")
    );
}

#[test]
fn toggle_favourite_flips_or_sets() {
    let h = Harness::new();
    let toggle = |favourite| Command::ClusterToggleFavourite {
        cluster: id("c"),
        favourite,
    };
    assert_eq!(
        run(&h, toggle(None)).unwrap(),
        ClusterCommandOutcome::Favourite(true)
    );
    assert!(h.entry("c").favourite);
    assert_eq!(
        run(&h, toggle(Some(false))).unwrap(),
        ClusterCommandOutcome::Favourite(false)
    );
    assert!(!h.entry("c").favourite);
}

#[test]
fn other_commands_are_refused_and_handles_says_which_are_ours() {
    let h = Harness::new();
    let error = run(&h, Command::PaletteToggle).expect_err("not a catalog command");
    assert_eq!(error.kind(), ErrorKind::Validation);
    assert!(ClusterCommands::handles(&Command::ClusterConnect {
        cluster: id("a")
    }));
    assert!(ClusterCommands::handles(&Command::ClusterDisconnect {
        cluster: id("a")
    }));
    assert!(ClusterCommands::handles(&Command::ClusterReconnect {
        cluster: id("a")
    }));
    assert!(ClusterCommands::handles(&Command::ClusterCancelConnect {
        cluster: id("a")
    }));
    assert!(ClusterCommands::handles(&Command::ClusterToggleFavourite {
        cluster: id("a"),
        favourite: None
    }));
    assert!(!ClusterCommands::handles(&Command::ClusterSelect {
        cluster: id("a")
    }));
    assert!(!ClusterCommands::handles(&Command::PaletteToggle));
}

#[test]
fn connecting_never_goes_through_the_mutation_path() {
    // Connect, disconnect and favourite are reads from the cluster's point of view: the
    // registry says so, which is what keeps them runnable on a read-only cluster.
    for command in [
        Command::ClusterConnect { cluster: id("a") },
        Command::ClusterDisconnect { cluster: id("a") },
        Command::ClusterReconnect { cluster: id("a") },
        Command::ClusterCancelConnect { cluster: id("a") },
        Command::ClusterToggleFavourite {
            cluster: id("a"),
            favourite: None,
        },
    ] {
        assert!(!command.is_mutating(), "{}", command.id());
    }
}
