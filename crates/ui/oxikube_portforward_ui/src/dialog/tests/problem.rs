//! Each `PortForwardError` / `StartError` becomes a message, a suggestion and (where one click
//! helps) a fix.

use std::io::ErrorKind;
use std::net::SocketAddr;

use oxikube_app::StartError;
use oxikube_domain::OxiError;
use oxikube_ports::PortForwardError;

use crate::dialog::{Fix, StartProblem};

fn addr(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

fn bind(port: u16, kind: ErrorKind) -> PortForwardError {
    PortForwardError::BindFailed {
        addr: addr(port),
        kind,
    }
}

#[test]
fn port_in_use_suggests_auto_and_offers_it() {
    let p = StartProblem::of_forward(&bind(8080, ErrorKind::AddrInUse));
    assert!(p.message.contains("already in use"), "{}", p.message);
    assert!(
        p.suggestion
            .unwrap()
            .contains("Use Auto or choose another local port")
    );
    assert_eq!(p.fix, Some(Fix::UseAuto));
}

#[test]
fn a_privileged_port_suggests_1024_or_more() {
    let p = StartProblem::of_forward(&bind(80, ErrorKind::PermissionDenied));
    assert!(p.suggestion.unwrap().contains("\u{2265} 1024"));
    assert_eq!(p.fix, Some(Fix::UseAuto));
}

#[test]
fn an_address_that_is_not_ours_offers_loopback() {
    let p = StartProblem::of_forward(&bind(8080, ErrorKind::AddrNotAvailable));
    assert_eq!(p.fix, Some(Fix::UseLoopback));
}

#[test]
fn forbidden_names_the_rbac_right() {
    let p = StartProblem::of_forward(&PortForwardError::Forbidden("pod shop/web".into()));
    assert!(
        p.suggestion
            .unwrap()
            .contains("`create` on `pods/portforward`")
    );
    assert_eq!(p.fix, None);
}

#[test]
fn a_pod_that_is_not_ready_says_to_wait() {
    let p = StartProblem::of_forward(&PortForwardError::PodNotReady("web".into()));
    assert!(p.suggestion.unwrap().contains("not ready"));
}

#[test]
fn the_other_errors_keep_the_ports_message_and_hint() {
    let closed = PortForwardError::PortClosed {
        port: 80,
        detail: "refused".into(),
    };
    let p = StartProblem::of_forward(&closed);
    assert!(p.message.contains("remote port 80"));
    assert!(p.suggestion.unwrap().contains("80"));
    let other = StartProblem::of_forward(&PortForwardError::Other("boom".into()));
    assert_eq!(other.message, "boom");
    assert_eq!(other.suggestion, None);
}

#[test]
fn read_only_and_not_connected_have_their_own_advice() {
    let ro = StartProblem::of_start(&StartError::ReadOnly("prod".into()));
    assert!(
        ro.suggestion
            .unwrap()
            .contains("portforward.allowed_in_readonly")
    );
    let off = StartProblem::of_start(&StartError::NotConnected("prod".into()));
    assert!(off.suggestion.unwrap().contains("Connect"));
}

#[test]
fn the_typed_error_survives_the_command_bus_error() {
    let error: OxiError = StartError::Forward(bind(8080, ErrorKind::AddrInUse)).into();
    assert_eq!(StartProblem::of(&error).fix, Some(Fix::UseAuto));
    // An error from elsewhere shows its own message and no fix.
    let plain = OxiError::internal("the main window is gone");
    let p = StartProblem::of(&plain);
    assert_eq!(p.message, "the main window is gone");
    assert_eq!(p.fix, None);
}
