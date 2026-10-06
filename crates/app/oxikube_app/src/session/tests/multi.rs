//! Several sessions at once, the user-controlled fields, and capabilities.

use oxikube_domain::ids::Scope;
use oxikube_domain::session::{ClusterSessionState, NamespaceSelection, SessionPhase, WatchScope};
use oxikube_domain::{Capabilities, Capability, ClusterColour, OxiError};
use oxikube_ports::HealthSignal;

use super::{Harness, id};
use crate::session::{SessionChange, SessionUpdate};

#[test]
fn two_sessions_do_not_share_state_and_updates_carry_their_id() {
    let mut h = Harness::new();
    let (a, b) = (id("a"), id("b"));
    h.connector
        .script()
        .connect
        .push_ok(())
        .push_err(OxiError::auth("login to b", false));
    assert_eq!(h.connect("a"), ClusterSessionState::Ready);
    assert_eq!(h.connect("b").phase(), SessionPhase::AuthRequired);

    h.manager.set_read_only(&b, true).unwrap();
    h.connector.report(&a, HealthSignal::Unhealthy);

    let sa = h.manager.get(&a).unwrap();
    let sb = h.manager.get(&b).unwrap();
    assert_eq!(sa.phase(), SessionPhase::Degraded);
    assert!(!sa.read_only());
    assert_eq!(sb.phase(), SessionPhase::AuthRequired);
    assert!(sb.read_only());
    assert_eq!(h.connector.live_connections(&a), 1);
    assert_eq!(h.connector.live_connections(&b), 0);
    let order: Vec<_> = h
        .manager
        .sessions()
        .iter()
        .map(|s| s.id().clone())
        .collect();
    assert_eq!(order, [a.clone(), b.clone()]);

    let updates = h.drain();
    let of = |cluster| -> Vec<SessionChange> {
        updates
            .iter()
            .filter(|u| &u.cluster == cluster)
            .map(|u| u.change.clone())
            .collect()
    };
    assert!(of(&a).contains(&SessionChange::StateChanged {
        from: SessionPhase::Ready,
        state: ClusterSessionState::Degraded
    }));
    assert!(!of(&a).contains(&SessionChange::ReadOnlyChanged(true)));
    assert!(of(&b).contains(&SessionChange::ReadOnlyChanged(true)));
    assert!(of(&b).contains(&SessionChange::StateChanged {
        from: SessionPhase::Connecting,
        state: ClusterSessionState::AuthRequired {
            reason: "login to b".into()
        }
    }));

    // Disconnecting one leaves the other alone.
    h.manager.disconnect(&a).unwrap();
    assert_eq!(
        h.manager.get(&b).unwrap().phase(),
        SessionPhase::AuthRequired
    );
}

#[test]
fn capabilities_reflect_a_fake_that_denies_mutation() {
    let h = Harness::new();
    let a = id("a");
    h.connector
        .ports_for(&a)
        .access
        .set_capabilities(Capabilities::all() - Capabilities::MUTATE);
    h.connect("a");
    let session = h.manager.get(&a).unwrap();
    assert!(!session.can(Capability::Mutate));
    assert!(session.can(Capability::Exec) && session.can(Capability::Logs));
    assert_eq!(
        session.capabilities(),
        Capabilities::all() - Capabilities::MUTATE
    );
    // The probe asks cluster-wide.
    assert_eq!(
        h.connector.ports_for(&a).access.recorded_calls(),
        [oxikube_testkit::AccessCall::Capabilities(None)]
    );
    // Capabilities are cleared on disconnect.
    h.manager.disconnect(&a).unwrap();
    assert_eq!(
        h.manager.get(&a).unwrap().capabilities(),
        Capabilities::empty()
    );
}

#[test]
fn namespace_read_only_and_colour_changes_are_announced_once() {
    let mut h = Harness::new();
    let a = id("a");
    h.connect("a");
    h.drain();

    let prod = NamespaceSelection::from_names(["prod", "staging"]);
    assert!(h.manager.set_namespace_selection(&a, prod.clone()).unwrap());
    assert!(!h.manager.set_namespace_selection(&a, prod.clone()).unwrap());
    assert!(h.manager.set_read_only(&a, true).unwrap());
    assert!(!h.manager.set_read_only(&a, true).unwrap());
    let red = Some(ClusterColour::rgb(255, 0, 0));
    assert!(h.manager.set_colour(&a, red).unwrap());
    assert!(!h.manager.set_colour(&a, red).unwrap());

    let update = |change| SessionUpdate {
        cluster: a.clone(),
        change,
    };
    assert_eq!(
        h.drain(),
        [
            update(SessionChange::NamespaceChanged(prod.clone())),
            update(SessionChange::ReadOnlyChanged(true)),
            update(SessionChange::ColourChanged(red)),
        ]
    );
    let session = h.manager.get(&a).unwrap();
    assert_eq!(
        session.watch_scope(Scope::Namespaced),
        WatchScope::Namespaces(vec!["prod".into(), "staging".into()])
    );
    assert_eq!(session.watch_scope(Scope::Cluster), WatchScope::Cluster);
    // The user's fields survive a disconnect.
    h.manager.disconnect(&a).unwrap();
    let session = h.manager.get(&a).unwrap();
    assert!(session.read_only());
    assert_eq!(session.colour(), red);
    assert_eq!(session.namespace_selection(), &prod);
}

#[test]
fn a_lagging_subscriber_is_told_how_much_it_missed() {
    use futures::{FutureExt, StreamExt};
    let h = Harness::with_config(crate::session::SessionManagerConfig {
        update_capacity: 2,
        ..Default::default()
    });
    let a = id("a");
    let mut slow = h.manager.subscribe();
    h.connect("a");
    let first = slow.next().now_or_never().unwrap().unwrap();
    assert_eq!(first, Err(crate::session::SessionLagged { missed: 2 }));
    assert_eq!(
        slow.next()
            .now_or_never()
            .unwrap()
            .unwrap()
            .unwrap()
            .cluster,
        a
    );
}
