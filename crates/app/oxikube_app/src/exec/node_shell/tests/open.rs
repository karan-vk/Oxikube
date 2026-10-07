//! Opening the terminal of a guarded node shell: the permit, the session, the leftover sweep and
//! the failures.

use std::time::Duration;

use futures::StreamExt as _;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{BackendEvent, NodeShellSpec};
use oxikube_testkit::{ExecPortCall, FakeTerminalBackend};

use super::{Fixture, custom_prefs};
use crate::exec::JANITOR_GRACE;

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    // The service's sweep has a timeout, which needs a runtime with a clock.
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("a runtime")
        .block_on(future)
}

#[test]
fn an_open_without_a_guarded_command_is_refused_and_creates_nothing() {
    let f = Fixture::new();
    let Err(err) = block_on(f.service.open_node_shell(&f.node())) else {
        panic!("nothing allowed this shell");
    };
    assert_eq!(err.kind(), ErrorKind::Forbidden);
    assert!(err.message().contains("node::Shell"), "{err}");
    assert!(f.exec.recorded_calls().is_empty(), "no pod, no sweep");
}

#[test]
fn a_permit_opens_exactly_one_terminal() {
    let f = Fixture::new();
    f.run().expect("runs");
    block_on(f.service.open_node_shell(&f.node())).expect("the first open");
    let Err(err) = block_on(f.service.open_node_shell(&f.node())) else {
        panic!("the permit is spent");
    };
    assert_eq!(err.kind(), ErrorKind::Forbidden);
}

#[test]
fn a_permit_is_for_its_node() {
    let f = Fixture::new();
    f.run().expect("runs");
    let other = crate::testing::node("a", "worker-2");
    assert!(block_on(f.service.open_node_shell(&other)).is_err());
    assert!(block_on(f.service.open_node_shell(&f.node())).is_ok());
}

#[test]
fn the_session_is_made_from_the_settings_template() {
    let f = Fixture::with(custom_prefs());
    f.run().expect("runs");
    block_on(f.service.open_node_shell(&f.node())).expect("opens");
    let calls = f.exec.recorded_calls();
    let ExecPortCall::NodeShell(spec) = calls.last().expect("a call") else {
        panic!("expected a node shell, got {calls:?}");
    };
    assert_eq!(spec.node, "worker-1");
    assert_eq!(spec.image, "registry.local/tools:2");
    assert_eq!(spec.namespace, "ops-debug");
    assert_eq!(spec.image_pull_secret.as_deref(), Some("regcred"));
    assert_eq!(spec.nsenter_args, ["-t", "1", "-m", "-n"]);
    assert_eq!(spec.max_lifetime, Duration::from_secs(900));
    assert_eq!(spec.labels.get("team").map(String::as_str), Some("infra"));
    assert_eq!(spec.tolerations, NodeShellSpec::new("x").tolerations);
}

#[test]
fn the_terminal_starts_with_a_line_naming_the_pod_namespace_and_image() {
    let f = Fixture::new();
    f.run().expect("runs");
    let backend = block_on(f.service.open_node_shell(&f.node())).expect("opens");
    let first = block_on(backend.output_stream().next()).expect("a first event");
    let BackendEvent::Output(bytes) = first else {
        panic!("expected output, got {first:?}");
    };
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("node shell on worker-1"), "{text:?}");
    assert!(
        text.contains("kube-system") && text.contains("busybox:1.37"),
        "{text:?}"
    );
    assert!(text.contains("deleted when this tab closes"), "{text:?}");
}

#[test]
fn the_first_open_sweeps_the_namespace_for_leftovers_once() {
    let f = Fixture::new();
    f.exec
        .script()
        .sweep_node_shells
        .push_ok(vec!["leftover-1".to_owned(), "leftover-2".to_owned()]);
    for _ in 0..2 {
        f.run().expect("runs");
        block_on(f.service.open_node_shell(&f.node())).expect("opens");
    }
    let calls = f.exec.recorded_calls();
    let sweeps: Vec<_> = calls
        .iter()
        .filter(|call| matches!(call, ExecPortCall::SweepNodeShells { .. }))
        .collect();
    assert_eq!(sweeps.len(), 1, "once per cluster and namespace: {calls:?}");
    assert_eq!(
        sweeps[0],
        &ExecPortCall::SweepNodeShells {
            namespace: "kube-system".into(),
            older_than: JANITOR_GRACE,
        }
    );
    assert!(
        matches!(calls[0], ExecPortCall::SweepNodeShells { .. }),
        "the sweep runs before the new pod exists: {calls:?}"
    );
}

#[test]
fn a_failed_sweep_does_not_stop_the_shell() {
    let f = Fixture::new();
    f.exec
        .script()
        .sweep_node_shells
        .push_err(OxiError::forbidden("pods is forbidden: cannot list"));
    f.run().expect("runs");
    assert!(block_on(f.service.open_node_shell(&f.node())).is_ok());
}

#[test]
fn a_pod_that_cannot_start_says_which_image_and_setting_to_check() {
    let f = Fixture::with(custom_prefs());
    f.run().expect("runs");
    f.exec.script().node_shell.push_err(OxiError::conflict(
        "shell of pod ops-debug/oxikube-node-shell-x cannot start: ImagePullBackOff",
    ));
    let Err(err) = block_on(f.service.open_node_shell(&f.node())) else {
        panic!("the pod cannot start");
    };
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(err.message().contains("registry.local/tools:2"), "{err}");
    assert!(err.message().contains("node_shell_image"), "{err}");
    assert!(err.message().contains("node_shell_pull_secret"), "{err}");
    assert!(
        err.message().contains("ImagePullBackOff"),
        "the cause stays: {err}"
    );
}

#[test]
fn a_slow_pull_is_a_timeout_that_says_so() {
    let f = Fixture::new();
    f.run().expect("runs");
    f.exec.script().node_shell.push_err(OxiError::timeout(
        "shell of pod x did not start within 120s",
    ));
    let Err(err) = block_on(f.service.open_node_shell(&f.node())) else {
        panic!("times out");
    };
    assert_eq!(err.kind(), ErrorKind::Timeout);
    assert!(err.message().contains("still be pulling"), "{err}");
    assert!(err.is_retryable(), "a timeout stays retryable");
}

#[test]
fn an_error_with_no_advice_passes_through_untouched() {
    let f = Fixture::new();
    f.run().expect("runs");
    f.exec
        .script()
        .node_shell
        .push_err(OxiError::network("the connection dropped"));
    let Err(err) = block_on(f.service.open_node_shell(&f.node())) else {
        panic!("fails");
    };
    assert_eq!(err.kind(), ErrorKind::Network);
    assert_eq!(err.message(), "the connection dropped");
}

#[test]
fn the_backend_is_the_ports_session_so_input_reaches_it() {
    let f = Fixture::new();
    f.run().expect("runs");
    let scripted = FakeTerminalBackend::silent();
    f.exec.script().node_shell.push_ok(scripted.clone());
    let backend = block_on(f.service.open_node_shell(&f.node())).expect("opens");
    block_on(backend.write(b"hostname\n")).expect("write");
    assert_eq!(scripted.written(), b"hostname\n");
}
