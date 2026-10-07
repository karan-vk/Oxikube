//! The lifecycle state machine and the banner texts, without a window.

use std::collections::HashSet;

use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::ExitStatus;

use super::*;

fn network() -> Failure {
    Failure::from_error(&OxiError::network("connection reset"))
}

fn run(local: bool, signals: impl IntoIterator<Item = Signal>) -> Lifecycle {
    signals
        .into_iter()
        .fold(Lifecycle::Connecting, |state, signal| {
            state.apply(signal, local)
        })
}

#[test]
fn a_pod_session_that_drops_is_disconnected_and_reconnects() {
    let state = run(false, [Signal::Started, Signal::Transport(network())]);
    assert_eq!(state, Lifecycle::Disconnected(network()));
    assert!(!state.accepts_input());
    assert!(state.can_relaunch());
    // The exit without a verdict that follows an error is only the stream ending.
    let state = state.apply(Signal::Exited(ExitStatus::default()), false);
    assert_eq!(
        state,
        Lifecycle::Disconnected(network()),
        "still disconnected"
    );
    let state = state.apply(Signal::Relaunch, false);
    assert_eq!(state, Lifecycle::Connecting);
    assert_eq!(state.apply(Signal::Started, false), Lifecycle::Running);
}

#[test]
fn a_stream_that_ends_without_a_verdict_is_a_dropped_connection() {
    let state = run(
        false,
        [Signal::Started, Signal::Exited(ExitStatus::default())],
    );
    assert_eq!(
        state,
        Lifecycle::Disconnected(Failure::new(FailureKind::StreamClosed))
    );
}

#[test]
fn an_exit_code_is_a_verdict_even_after_a_transport_error() {
    let state = run(
        false,
        [
            Signal::Started,
            Signal::Transport(network()),
            Signal::Exited(ExitStatus::with_code(137)),
        ],
    );
    assert_eq!(state, Lifecycle::Exited(ExitStatus::with_code(137)));
}

#[test]
fn a_local_shell_has_no_connection_to_lose() {
    let state = run(true, [Signal::Started, Signal::Transport(network())]);
    assert_eq!(
        state,
        Lifecycle::Running,
        "a read error alone changes nothing"
    );
    let state = state.apply(Signal::Exited(ExitStatus::default()), true);
    assert_eq!(state, Lifecycle::Exited(ExitStatus::default()));
    assert_eq!(state.apply(Signal::Relaunch, true), Lifecycle::Connecting);
}

#[test]
fn a_failed_start_can_be_retried() {
    let failure = Failure::from_error(&OxiError::forbidden("no"));
    let state = run(false, [Signal::StartFailed(failure.clone())]);
    assert_eq!(state, Lifecycle::Failed(failure));
    assert_eq!(state.apply(Signal::Relaunch, false), Lifecycle::Connecting);
}

#[test]
fn a_running_terminal_does_not_relaunch_and_a_closed_one_stays_closed() {
    let running = Lifecycle::Running;
    assert!(!running.can_relaunch());
    assert_eq!(running.clone().apply(Signal::Relaunch, false), running);
    assert!(running.accepts_input());
    let closed = running.apply(Signal::Close, false);
    assert_eq!(closed, Lifecycle::Closed);
    for signal in [
        Signal::Started,
        Signal::Relaunch,
        Signal::Exited(ExitStatus::success()),
    ] {
        assert_eq!(closed.clone().apply(signal, false), Lifecycle::Closed);
    }
    assert_eq!(
        Lifecycle::Connecting.apply(Signal::Close, true),
        Lifecycle::Closed
    );
}

#[test]
fn a_pod_command_that_could_not_run_is_an_exit_with_its_message() {
    let status = ExitStatus {
        message: Some("exec: \"vim\": executable file not found".into()),
        ..ExitStatus::default()
    };
    let state = run(false, [Signal::Started, Signal::Exited(status.clone())]);
    assert_eq!(state, Lifecycle::Exited(status));
    let banner = state.banner(false).expect("a banner");
    assert_eq!(banner.headline, "The command could not run");
    assert!(banner.detail.contains("executable file not found"));
}

#[test]
fn no_banner_while_it_runs_starts_or_is_closed() {
    for state in [Lifecycle::Connecting, Lifecycle::Running, Lifecycle::Closed] {
        assert_eq!(state.banner(false), None);
        assert_eq!(state.banner(true), None);
    }
}

#[test]
fn a_dropped_connection_banner_offers_reconnect_and_says_the_shell_state_is_gone() {
    let banner = Lifecycle::Disconnected(network())
        .banner(false)
        .expect("a banner");
    assert_eq!(banner.headline, "Connection lost");
    assert_eq!(banner.actions, [BannerAction::Reconnect]);
    assert_eq!(banner.tone, Tone::Warning);
    assert!(banner.detail.contains("state of the old shell is gone"));
    assert!(
        banner.detail.contains("connection reset"),
        "the adapter's message"
    );
}

#[test]
fn a_local_shell_exit_banner_shows_the_code_and_offers_restart() {
    let failed = Lifecycle::Exited(ExitStatus::with_code(2))
        .banner(true)
        .expect("a banner");
    assert_eq!(failed.headline, "Shell exited with code 2");
    assert_eq!(
        failed.actions,
        [BannerAction::Restart, BannerAction::CloseTab]
    );
    assert_eq!(failed.tone, Tone::Warning);

    let clean = Lifecycle::Exited(ExitStatus::success())
        .banner(true)
        .expect("a banner");
    assert_eq!(clean.headline, "Shell exited with code 0");
    assert_eq!(
        clean.actions,
        [BannerAction::CloseTab, BannerAction::Restart],
        "closing is the primary action after a clean exit"
    );
    assert_eq!(clean.tone, Tone::Info);

    let killed = Lifecycle::Exited(ExitStatus::killed_by("KILL"))
        .banner(true)
        .expect("a banner");
    assert_eq!(killed.headline, "Shell ended by signal KILL");
}

#[test]
fn a_pod_session_that_ended_reconnects_not_restarts() {
    let banner = Lifecycle::Exited(ExitStatus::with_code(1))
        .banner(false)
        .expect("a banner");
    assert_eq!(banner.headline, "Session ended with code 1");
    assert_eq!(
        banner.actions,
        [BannerAction::Reconnect, BannerAction::CloseTab]
    );
}

#[test]
fn a_failed_start_offers_the_action_of_its_kind_of_terminal() {
    let remote = Lifecycle::Failed(Failure::from_error(&OxiError::not_found(
        "pod a/b not found",
    )));
    assert_eq!(
        remote.banner(false).expect("banner").actions,
        [BannerAction::Reconnect]
    );
    let local = Lifecycle::Failed(Failure::local_start(&OxiError::validation("no such shell")));
    let banner = local.banner(true).expect("banner");
    assert_eq!(banner.actions, [BannerAction::Restart]);
    assert_eq!(banner.tone, Tone::Error);
    assert_eq!(banner.headline, "The terminal could not start");
    assert!(banner.detail.contains("no such shell"));
}

/// Every kind of error a pod terminal can hit says something different and specific, never the
/// generic "stream closed".
#[test]
fn each_error_kind_has_its_own_banner_text() {
    let cases = [
        (
            ErrorKind::Auth,
            FailureKind::AuthExpired,
            "Authentication expired",
        ),
        (
            ErrorKind::Forbidden,
            FailureKind::Forbidden,
            "Not allowed to open a terminal here",
        ),
        (
            ErrorKind::NotFound,
            FailureKind::PodGone,
            "The pod or container is gone",
        ),
        (
            ErrorKind::Conflict,
            FailureKind::ContainerStopped,
            "The container is not running",
        ),
        (
            ErrorKind::Network,
            FailureKind::ConnectionLost,
            "Connection lost",
        ),
        (
            ErrorKind::Timeout,
            FailureKind::ConnectionLost,
            "Connection lost",
        ),
        (
            ErrorKind::Unsupported,
            FailureKind::Unsupported,
            "The cluster cannot open this terminal",
        ),
        (
            ErrorKind::Validation,
            FailureKind::Rejected,
            "The cluster refused the request",
        ),
        (
            ErrorKind::Internal,
            FailureKind::Unexpected,
            "The terminal failed",
        ),
        (
            ErrorKind::BudgetExceeded,
            FailureKind::Unexpected,
            "The terminal failed",
        ),
    ];
    assert_eq!(cases.len(), ErrorKind::ALL.len(), "every kind is mapped");
    let mut headlines = HashSet::new();
    let mut hints = HashSet::new();
    for (error_kind, kind, headline) in cases {
        let failure = Failure::from_error(&OxiError::new(error_kind, "adapter message"));
        assert_eq!(failure.kind(), kind, "{error_kind:?}");
        assert_eq!(failure.headline(), headline, "{error_kind:?}");
        assert_eq!(failure.detail(), Some("adapter message"));
        headlines.insert(failure.headline());
        hints.insert(failure.hint());
        for text in [failure.headline(), failure.hint()] {
            assert!(!text.to_lowercase().contains("eof"), "{text}");
        }
    }
    assert_eq!(headlines.len(), 8, "one headline per distinct cause");
    assert_eq!(hints.len(), 8);
}

#[test]
fn the_expired_login_banner_says_how_to_recover() {
    let banner = Lifecycle::Disconnected(Failure::from_error(&OxiError::auth("expired", true)))
        .banner(false)
        .expect("banner");
    assert_eq!(banner.headline, "Authentication expired");
    assert!(banner.detail.contains("Refresh the login"));
}

#[test]
fn details_are_redacted_and_never_carry_output() {
    let failure = Failure::from_error(&OxiError::network(
        "dial failed with Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.abc.def",
    ));
    let shown = Lifecycle::Disconnected(failure)
        .banner(false)
        .expect("banner");
    assert!(!shown.detail.contains("eyJhbGci"), "{}", shown.detail);
}
