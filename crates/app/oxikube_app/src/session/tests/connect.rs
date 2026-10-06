//! Happy path, lookups and the shape of a connected session.

use futures::FutureExt;
use oxikube_domain::session::{ClusterSessionState, NamespaceSelection, SessionPhase};
use oxikube_domain::{Capabilities, ClusterColour, ErrorKind};
use oxikube_ports::ExecInteractivity;
use oxikube_testkit::ConnectorCall;

use super::{Harness, ctx, id};
use crate::session::{SessionChange, SessionOptions, SessionUpdate};

#[test]
fn connect_opens_from_the_catalog_and_reaches_ready() {
    let mut h = Harness::new();
    assert_eq!(h.connect("a"), ClusterSessionState::Ready);

    let updates = h.drain();
    let a = id("a");
    let changes: Vec<_> = updates.iter().map(|u| &u.change).collect();
    assert!(updates.iter().all(|u| u.cluster == a));
    assert_eq!(
        changes,
        [
            &SessionChange::Opened,
            &SessionChange::StateChanged {
                from: SessionPhase::Disconnected,
                state: ClusterSessionState::Connecting
            },
            &SessionChange::CapabilitiesChanged(Capabilities::all()),
            &SessionChange::StateChanged {
                from: SessionPhase::Connecting,
                state: ClusterSessionState::Ready
            },
        ]
    );

    let session = h.manager.get(&a).unwrap();
    assert_eq!(session.context().as_str(), "a");
    assert!(session.is_connected());
    assert_eq!(session.capabilities(), Capabilities::all());
    assert!(session.resources().is_some() && session.discovery().is_some());
    assert!(session.tables().is_some() && session.logs().is_some());
    assert!(session.exec().is_some() && session.port_forward().is_some());
    assert!(session.metrics().is_some() && session.access().is_some());
    assert_eq!(h.connector.live_connections(&a), 1);
}

#[test]
fn ready_is_announced_without_opening_any_feed() {
    let h = Harness::new();
    h.connect("a");
    let ports = h.connector.ports_for(&id("a"));
    assert_eq!(ports.discovery.recorded_calls().len(), 1);
    assert_eq!(ports.access.recorded_calls().len(), 1);
    assert!(ports.resources.recorded_calls().is_empty());
    assert!(ports.tables.recorded_calls().is_empty());
    assert!(ports.logs.recorded_calls().is_empty());
}

#[test]
fn unknown_cluster_is_not_found() {
    let h = Harness::new();
    let err = h
        .manager
        .connect(&id("nope"))
        .now_or_never()
        .unwrap()
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(h.manager.sessions().is_empty());
    assert_eq!(
        h.manager.disconnect(&id("nope")).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    assert!(h.manager.set_read_only(&id("nope"), true).is_err());
}

#[test]
fn catalog_errors_surface_from_connect() {
    let h = Harness::new();
    h.source
        .script()
        .contexts
        .push_err(oxikube_domain::OxiError::validation("bad kubeconfig"));
    let err = h
        .manager
        .connect(&id("a"))
        .now_or_never()
        .unwrap()
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
}

#[test]
fn connect_is_idempotent_while_connected() {
    let mut h = Harness::new();
    h.connect("a");
    h.drain();
    assert_eq!(h.connect("a"), ClusterSessionState::Ready);
    assert!(h.drain().is_empty());
    assert_eq!(h.connector.recorded_calls().len(), 1);
    assert_eq!(h.connector.live_connections(&id("a")), 1);
}

#[test]
fn open_uses_options_and_connect_passes_the_exec_policy() {
    let mut h = Harness::new();
    let options = SessionOptions {
        namespace_selection: NamespaceSelection::single("prod"),
        read_only: true,
        colour: Some(ClusterColour::rgb(0xe5, 0x48, 0x4d)),
        exec_interactivity: ExecInteractivity::IfAvailable,
    };
    let session = h.manager.open(&ctx("a"), options.clone());
    assert_eq!(session.phase(), SessionPhase::Disconnected);
    assert!(session.read_only());
    assert_eq!(
        session.namespace_selection(),
        &NamespaceSelection::single("prod")
    );
    assert_eq!(session.colour(), options.colour);
    assert!(session.resources().is_none());
    assert_eq!(session.capabilities(), Capabilities::empty());

    // Opening again keeps the existing session and sends nothing.
    h.manager.open(&ctx("a"), SessionOptions::default());
    assert!(h.manager.get(&id("a")).unwrap().read_only());
    assert_eq!(
        h.drain(),
        [SessionUpdate {
            cluster: id("a"),
            change: SessionChange::Opened
        }]
    );

    h.connect("a");
    assert_eq!(
        h.connector.recorded_calls(),
        [ConnectorCall::Connect {
            cluster: id("a"),
            context: ctx("a").context,
            exec_interactivity: ExecInteractivity::IfAvailable,
        }]
    );
    assert!(
        h.manager
            .set_exec_interactivity(&id("a"), ExecInteractivity::Never)
            .unwrap()
    );
    assert_eq!(
        h.manager.get(&id("a")).unwrap().exec_interactivity(),
        ExecInteractivity::Never
    );
}

#[test]
fn close_disconnects_and_forgets() {
    let mut h = Harness::new();
    h.connect("a");
    h.drain();
    assert!(h.manager.close(&id("a")));
    assert!(!h.manager.close(&id("a")));
    assert!(h.manager.get(&id("a")).is_none());
    assert_eq!(h.connector.live_connections(&id("a")), 0);
    let changes: Vec<_> = h.drain().into_iter().map(|u| u.change).collect();
    assert_eq!(
        changes,
        [
            SessionChange::CapabilitiesChanged(Capabilities::empty()),
            SessionChange::StateChanged {
                from: SessionPhase::Ready,
                state: ClusterSessionState::Disconnected
            },
            SessionChange::Closed,
        ]
    );
}

#[test]
fn update_stream_ends_when_the_manager_is_dropped() {
    let mut h = Harness::new();
    let manager = std::mem::replace(
        &mut h.manager,
        crate::session::ClusterSessionManager::new(
            h.connector.clone(),
            h.source.clone(),
            h.clock.clone(),
        ),
    );
    drop(manager);
    use futures::StreamExt;
    assert_eq!(h.updates.next().now_or_never(), Some(None));
}

#[test]
fn connect_runs_on_any_worker_thread() {
    fn assert_send<T: Send + 'static>(_: &T) {}
    fn assert_sync<T: Send + Sync>() {}
    assert_sync::<crate::session::ClusterSessionManager>();
    assert_sync::<crate::session::ClusterSession>();
    let h = Harness::new();
    let manager = h.manager;
    let connect = async move { manager.reconnect(&id("a")).await };
    // What `spawn_kube` requires of the future it is given.
    assert_send(&connect);
    assert_eq!(
        connect.now_or_never().unwrap().unwrap(),
        ClusterSessionState::Ready
    );
}
