//! One store per connected session.

use std::sync::Arc;

use crate::store::{ResourceStores, StoreRuntime};
use crate::testing::{Harness as BusHarness, id};

use super::Executor;

#[test]
fn stores_follow_the_session_connection() {
    let bus = BusHarness::new();
    let exec = Executor::default();
    let stores = ResourceStores::new(StoreRuntime {
        spawner: exec.spawner(),
        clock: Arc::new(oxikube_testkit::FakeClockPort::default()),
        probe: None,
    });

    bus.manager.open(
        &crate::testing::cluster_context("a"),
        crate::session::SessionOptions::default(),
    );
    let session = bus.manager.get(&id("a")).expect("open");
    assert!(stores.for_session(&session).is_none(), "not connected yet");

    bus.connect("a", false);
    let session = bus.manager.get(&id("a")).expect("connected");
    let store = stores.for_session(&session).expect("a store");
    assert_eq!(store.cluster(), &id("a"));
    let again = stores.for_session(&session).expect("the same store");
    assert!(Arc::ptr_eq(
        &store.ports().resources,
        &again.ports().resources
    ));
    assert_eq!(stores.clusters(), [id("a")]);

    bus.manager.disconnect(&id("a")).expect("disconnect");
    let session = bus.manager.get(&id("a")).expect("still open");
    assert!(stores.for_session(&session).is_none());
    assert!(stores.clusters().is_empty(), "forgotten while disconnected");

    bus.connect("a", false);
    let session = bus.manager.get(&id("a")).expect("reconnected");
    assert!(stores.for_session(&session).is_some());
    assert!(stores.remove(&id("a")));
    assert!(!stores.remove(&id("a")));
}
