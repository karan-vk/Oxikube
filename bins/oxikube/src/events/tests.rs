//! The events glue in the app over fakes: the session follower runs on the foreground executor,
//! so a started app with no connected cluster has nothing on the Tokio bridge (the startup
//! scenario waits for an empty one), and only a cluster's feed goes there.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::store::Spawner;
use oxikube_app::{EventFilter, EventsState};
use oxikube_domain::ids::ClusterId;
use oxikube_testkit::TestPorts;

use super::start_on;
use crate::app_state::AppState;
use oxikube_testkit::FakeClockPort;

fn cluster() -> ClusterId {
    TestPorts::cluster_id()
}

/// A spawner onto the test's background executor that counts what it is given.
fn counting(cx: &TestAppContext) -> (Arc<dyn Spawner>, Arc<AtomicUsize>) {
    let spawned = Arc::new(AtomicUsize::new(0));
    let executor = cx.executor();
    let count = spawned.clone();
    let spawner: Arc<dyn Spawner> = Arc::new(move |task| {
        count.fetch_add(1, Ordering::SeqCst);
        executor.spawn(task).detach();
    });
    (spawner, spawned)
}

#[gpui::test]
fn start_puts_nothing_on_the_bridge_until_a_cluster_feed_runs(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    let (spawner, spawned) = counting(cx);
    let state = cx.update(|cx| {
        let state = AppState::test_with(cx, &ports);
        let clock = Arc::new(FakeClockPort::default());
        start_on(&state.services().events, clock.clone(), spawner.clone(), cx);
        // A second window's mount reuses what the first started.
        start_on(&state.services().events, clock, spawner, cx);
        state
    });
    cx.run_until_parked();
    assert_eq!(
        spawned.load(Ordering::SeqCst),
        0,
        "an idle app keeps no events task on the Tokio bridge"
    );

    let services = state.services();
    let subscription = services
        .events
        .subscribe(&cluster(), EventFilter::warnings());
    block_on(services.sessions.connect(&cluster())).expect("connect");
    cx.run_until_parked();
    assert!(
        services.events.is_watching(&cluster()),
        "the foreground follower opened the feed on connect"
    );
    assert!(
        spawned.load(Ordering::SeqCst) > 0,
        "the feed runs on the bridge"
    );

    services
        .sessions
        .disconnect(&cluster())
        .expect("disconnect");
    cx.run_until_parked();
    assert_eq!(
        services.events.state(&cluster()),
        EventsState::Idle,
        "the follower stopped the feed on disconnect"
    );
    drop(subscription);
}
