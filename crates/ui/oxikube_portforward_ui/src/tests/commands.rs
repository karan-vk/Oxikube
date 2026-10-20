//! `portforward::CopyUrl`, `OpenInBrowser` and `TogglePanel` on the bus: registered with their
//! tools, checked against the forward, queued for the window.

use oxikube_app::command_bus::{CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::ErrorKind;
use oxikube_domain::command::{self, Command};

use super::{FakeSource, active, forward, prod};
use crate::commands::{COMMANDS, PortForwardUiSink, UiRequest, register_commands, run};

fn setup() -> (
    std::sync::Arc<FakeSource>,
    PortForwardUiSink,
    futures::channel::mpsc::UnboundedReceiver<UiRequest>,
) {
    let source = FakeSource::new(vec![
        active(forward(1, prod(), "Pod", "web"), 18080),
        forward(2, prod(), "Pod", "idle"),
    ]);
    let (sink, rx) = PortForwardUiSink::channel();
    (source, sink, rx)
}

#[test]
fn copy_and_open_hand_the_forwards_url_to_the_window() {
    let (source, sink, mut rx) = setup();
    let copied = run(&*source, &sink, Command::PortForwardCopyUrl { id: 1 }).expect("copy");
    assert_eq!(
        copied.message.as_deref(),
        Some("Copied http://127.0.0.1:18080")
    );
    let opened = run(&*source, &sink, Command::PortForwardOpenInBrowser { id: 1 }).expect("open");
    assert_eq!(opened.data.unwrap()["url"], "http://127.0.0.1:18080");
    assert_eq!(
        rx.try_recv().ok(),
        Some(UiRequest::CopyUrl("http://127.0.0.1:18080".into()))
    );
    assert_eq!(
        rx.try_recv().ok(),
        Some(UiRequest::OpenUrl("http://127.0.0.1:18080".into()))
    );
}

#[test]
fn a_forward_without_an_address_or_an_unknown_one_is_an_error_not_a_request() {
    let (source, sink, mut rx) = setup();
    let unbound = run(&*source, &sink, Command::PortForwardCopyUrl { id: 2 }).unwrap_err();
    assert_eq!(unbound.kind(), ErrorKind::Conflict);
    assert!(
        unbound.message().contains("not listening yet"),
        "{}",
        unbound.message()
    );
    let unknown = run(&*source, &sink, Command::PortForwardOpenInBrowser { id: 9 }).unwrap_err();
    assert_eq!(unknown.kind(), ErrorKind::NotFound);
    assert!(rx.try_recv().is_err(), "nothing was queued");
}

#[test]
fn the_toggle_carries_whether_it_only_shows() {
    let (source, sink, mut rx) = setup();
    run(
        &*source,
        &sink,
        Command::PortForwardTogglePanel { show: true },
    )
    .unwrap();
    run(
        &*source,
        &sink,
        Command::PortForwardTogglePanel { show: false },
    )
    .unwrap();
    assert_eq!(rx.try_recv().ok(), Some(UiRequest::Toggle { show: true }));
    assert_eq!(rx.try_recv().ok(), Some(UiRequest::Toggle { show: false }));
}

#[test]
fn a_gone_window_is_an_error() {
    let (source, sink, rx) = setup();
    drop(rx);
    let err = run(
        &*source,
        &sink,
        Command::PortForwardTogglePanel { show: true },
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Internal);
}

#[test]
fn every_command_is_registered_with_its_tool_and_changes_no_cluster() {
    let (source, sink, _rx) = setup();
    let mut registry = CommandRegistry::new();
    register_commands(&mut registry, source, sink).expect("registered");
    for id in COMMANDS {
        let meta = *command::lookup(id).unwrap();
        assert!(!meta.mutating, "{id} changes no cluster");
        assert!(id.tool_name().starts_with("k8s.portforward_"), "{id}");
        let again = registry.register(meta, |_: Command, _: HandlerContext| async {
            Ok(oxikube_app::command_bus::CommandOutput::default())
        });
        assert!(
            matches!(again, Err(RegisterError::Duplicate { .. })),
            "{id} is taken"
        );
    }
}
