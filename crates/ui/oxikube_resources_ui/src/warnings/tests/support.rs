//! The events index in a test window: [`Fixture::install_events`].

use std::sync::Arc;
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use oxikube_app::store::Spawner;
use oxikube_app::{BadgeTrack, EventService, EventsConfig, WarningBadges};
use oxikube_domain::event::Event;
use oxikube_ports::ClockPort;
use oxikube_testkit::{FakeEventFeed, FakeEventsPort, TICK, event};

use crate::table::tests::fixture::{Fixture, cluster};
use crate::warnings::ObjectEvents;

/// A `Warning` event `uid` about the pod `ns/name` (uid `pod_uid`), last seen `age` ago.
pub(crate) fn warning_event(
    uid: &str,
    ns: &str,
    name: &str,
    pod_uid: &str,
    reason: &str,
    age: SignedDuration,
) -> Event {
    let mut e = event()
        .uid(uid)
        .warning()
        .reason(reason)
        .message(&format!("{reason} for {name}"))
        .regarding("Pod", Some(ns), name, pod_uid)
        .build();
    e.last_seen = Some(Timestamp::now() - age);
    e.first_seen = e.last_seen;
    e
}

/// The app's events index over fakes, installed as the views' `ObjectEvents`.
pub(crate) struct EventsRig {
    pub(crate) badges: WarningBadges,
    pub(crate) service: EventService,
    pub(crate) port: Arc<FakeEventsPort>,
    _track: BadgeTrack,
}

impl EventsRig {
    /// Plays the next tick of the scripted feed.
    pub(crate) fn tick(&self) {
        self.port.clock().advance(TICK);
    }
}

impl Fixture {
    /// Makes an `EventService` over the fixture's sessions and the cluster's fake events port,
    /// installs it as `ObjectEvents`, and tracks the cluster as its tab does. Call it before the
    /// cluster connects and before any table or drawer opens: a view built earlier has none.
    pub(crate) fn install_events(&mut self, feed: FakeEventFeed) -> EventsRig {
        let port = self.ports().events;
        port.install(feed);
        let clock: Arc<dyn ClockPort> = self.clock.clone();
        let service = EventService::new(
            self.sessions.clone(),
            clock.clone(),
            EventsConfig::default(),
        );
        let spawner: Arc<dyn Spawner> = self.vcx.update(|_, cx| {
            let executor = cx.background_executor().clone();
            Arc::new(move |task| executor.spawn(task).detach())
        });
        service.start(spawner.clone());
        let badges =
            WarningBadges::new(service.clone(), clock, spawner, Duration::from_secs(3_600));
        let track = badges.track(&cluster());
        let installed = badges.clone();
        self.vcx
            .update(|_, cx| ObjectEvents::install(installed, cx));
        EventsRig {
            badges,
            service,
            port,
            _track: track,
        }
    }
}
