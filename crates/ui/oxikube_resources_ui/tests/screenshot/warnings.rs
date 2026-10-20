//! The warning badge column and the Events tab (E14-S04): a pods table whose `api`, `cart` and
//! `payments` rows have a recent `Warning` event show the icon in the second column, in the dark
//! and the light theme; a crash-looping pod's Events tab lists its Warning and Normal events
//! newest first with the Type word, the reason, the count and the age.
//!
//! The badge reads the real clock (a warning is recent when it is within the hour of "now"), so
//! the table's events are seen now; the Events tab pins "now" and uses fixed times, so its ages do
//! not move.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, px, size};
use jiff::Timestamp;
use oxikube_app::store::{ResourceStores, Spawner};
use oxikube_app::{ClusterSessionManager, CoreColumns, EventService, EventsConfig, WarningBadges};
use oxikube_domain::Resource;
use oxikube_domain::event::Event;
use oxikube_domain::ids::{ClusterId, ContextName, ResourceRef};
use oxikube_ports::{ClockPort, ClusterContext, SourceId, StatePort as _};
use oxikube_resources_ui::detail::{DetailDeps, DetailTab, DetailView, Mount};
use oxikube_resources_ui::table::{
    ColumnPrefs, ResourceTable, ResourceTableDeps, prefs_key, store_runtime,
};
use oxikube_resources_ui::warnings::ObjectEvents;
use oxikube_testkit::screenshot::RgbaImage;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeEventFeed, FakeStatePort,
    TICK, event, pod,
};

use super::{HEIGHT, Ignore, WIDTH, check, headless, pods_kind};

const TAB_WIDTH: f32 = 480.0;
const TAB_HEIGHT: f32 = 420.0;

fn pod_of(name: &str, uid: &str) -> oxikube_testkit::PodBuilder {
    pod()
        .namespace("shop")
        .name(name)
        .uid(uid)
        .node("kind-worker")
}

/// A `Warning` event about `shop/name` (uid `uid`), last seen at `at`.
fn warning(
    uid: &str,
    name: &str,
    pod_uid: &str,
    reason: &str,
    message: &str,
    at: Timestamp,
) -> Event {
    let mut e = event()
        .uid(uid)
        .warning()
        .reason(reason)
        .message(message)
        .regarding("Pod", Some("shop"), name, pod_uid)
        .build();
    e.last_seen = Some(at);
    e.first_seen = Some(at);
    e
}

struct Rig {
    sessions: ClusterSessionManager,
    cluster: ClusterId,
    clock: Arc<FakeClockPort>,
    feed_port: Arc<oxikube_testkit::FakeEventsPort>,
}

/// A connected cluster serving `pods`, with `feed` as its events.
fn rig(pods: Vec<Resource>, feed: FakeEventFeed) -> Rig {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let ports = connector.ports_for(&cluster);
    for resource in pods {
        ports.resources.insert(resource);
    }
    ports.events.install(feed);
    let clock = Arc::new(FakeClockPort::default());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    Rig {
        sessions,
        cluster,
        clock,
        feed_port: ports.events,
    }
}

/// The events index over `rig`, installed as the views' `ObjectEvents`; connects the cluster.
fn install_events(
    rig: &Rig,
    cx: &mut gpui::HeadlessAppContext,
) -> anyhow::Result<oxikube_app::BadgeTrack> {
    let clock: Arc<dyn ClockPort> = rig.clock.clone();
    let service = EventService::new(rig.sessions.clone(), clock.clone(), EventsConfig::default());
    let executor = cx.background_executor().clone();
    let spawner: Arc<dyn Spawner> = Arc::new(move |task| executor.spawn(task).detach());
    service.start(spawner.clone());
    let badges = WarningBadges::new(
        service,
        clock,
        spawner,
        std::time::Duration::from_secs(3_600),
    );
    let track = badges.track(&rig.cluster);
    cx.update(|cx| ObjectEvents::install(badges, cx));
    futures::executor::block_on(rig.sessions.connect(&rig.cluster))?;
    cx.run_until_parked();
    rig.feed_port.clock().advance(TICK);
    cx.run_until_parked();
    Ok(track)
}

fn table(light: bool) -> anyhow::Result<RgbaImage> {
    let now = Timestamp::now();
    let pods = vec![
        pod_of("api-7c9d", "uid-api")
            .running()
            .ip("10.244.0.12")
            .build(),
        pod_of("cart-5f6b", "uid-cart").pending().build(),
        pod_of("payments-9e1c", "uid-payments")
            .crash_loop()
            .restarts(14)
            .ip("10.244.0.31")
            .build(),
        pod_of("web-6a4f", "uid-web")
            .running()
            .ip("10.244.0.7")
            .build(),
    ];
    let minutes = |m: i64| now - jiff::SignedDuration::from_mins(m);
    let feed = FakeEventFeed::new().initial([
        warning(
            "e1",
            "api-7c9d",
            "uid-api",
            "Unhealthy",
            "Readiness probe failed",
            minutes(3),
        ),
        warning(
            "e2",
            "cart-5f6b",
            "uid-cart",
            "FailedScheduling",
            "0/1 nodes are available",
            minutes(12),
        ),
        warning(
            "e3",
            "payments-9e1c",
            "uid-payments",
            "BackOff",
            "Back-off restarting failed container",
            minutes(1),
        ),
        // A warning from yesterday: past the hour, no badge.
        warning(
            "e4",
            "web-6a4f",
            "uid-web",
            "Failed",
            "Error: ImagePullBackOff",
            minutes(26 * 60),
        ),
    ]);
    let rig = rig(pods, feed);
    let state = Arc::new(FakeStatePort::new());
    let layout = ColumnPrefs {
        // Ages and last-restart times move with the clock.
        visible: [("age".to_owned(), false), ("restarts".to_owned(), false)].into(),
        ..ColumnPrefs::default()
    };
    futures::executor::block_on(state.kv_set(
        &prefs_key(&pods_kind().gvk).expect("key"),
        serde_json::to_value(layout)?,
    ))?;
    let mut cx = headless();
    let _track = install_events(&rig, &mut cx)?;
    let Rig {
        sessions,
        cluster,
        clock,
        ..
    } = rig;
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        let tokens = if light {
            oxikube_ui::Tokens::light()
        } else {
            oxikube_ui::Tokens::dark()
        };
        oxikube_ui::set_tokens(cx, tokens);
        oxikube_runtime::init_deterministic(cx);
        cx.set_reduce_motion(true);
        let clock: Arc<dyn ClockPort> = clock;
        let deps = ResourceTableDeps {
            sessions,
            stores: Arc::new(ResourceStores::new(store_runtime(clock, cx))),
            columns: Arc::new(CoreColumns::new()),
            state,
            dispatcher: Rc::new(Ignore),
            actions: None,
        };
        cx.new(|cx| ResourceTable::new(cluster, pods_kind(), deps, window, cx))
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |view, _, cx| {
        let table = view.downcast::<ResourceTable>().expect("the root view");
        table.update(cx, |table, cx| table.move_cursor(1, false, cx));
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.capture_screenshot(window.into())
}

fn events_tab(light: bool) -> anyhow::Result<RgbaImage> {
    let at = |s: &str| -> Timestamp { s.parse().expect("a timestamp") };
    let crashing = {
        let mut pod = pod_of("payments-9e1c", "uid-payments")
            .crash_loop()
            .restarts(14)
            .ip("10.244.0.31")
            .created("2026-01-09T12:00:00Z")
            .json();
        pod["metadata"]["resourceVersion"] = serde_json::json!("9");
        Resource::from_json(pod)?
    };
    let mut normal = event()
        .uid("e0")
        .reason("Pulled")
        .message("Container image \"payments:1.4\" already present on machine")
        .regarding("Pod", Some("shop"), "payments-9e1c", "uid-payments")
        .build();
    normal.last_seen = Some(at("2026-01-09T23:40:00Z"));
    normal.first_seen = normal.last_seen;
    let mut back_off = warning(
        "e1",
        "payments-9e1c",
        "uid-payments",
        "BackOff",
        "Back-off restarting failed container payments in pod payments-9e1c",
        at("2026-01-09T23:58:00Z"),
    );
    back_off.count = 42;
    let probe = warning(
        "e2",
        "payments-9e1c",
        "uid-payments",
        "Unhealthy",
        "Liveness probe failed: HTTP probe failed with statuscode: 500",
        at("2026-01-09T23:50:00Z"),
    );
    let rig = rig(
        vec![crashing],
        FakeEventFeed::new().initial([normal, probe, back_off]),
    );
    let mut cx = headless();
    let _track = install_events(&rig, &mut cx)?;
    let Rig {
        sessions,
        cluster,
        clock,
        ..
    } = rig;
    let target = ResourceRef::namespaced(cluster, pods_kind().gvk, "shop", "payments-9e1c");
    let window = cx.open_window(size(px(TAB_WIDTH), px(TAB_HEIGHT)), |_, cx| {
        oxikube_ui::init(cx);
        let tokens = if light {
            oxikube_ui::Tokens::light()
        } else {
            oxikube_ui::Tokens::dark()
        };
        oxikube_ui::set_tokens(cx, tokens);
        oxikube_runtime::init_deterministic(cx);
        cx.set_reduce_motion(true);
        let clock: Arc<dyn ClockPort> = clock;
        let deps = DetailDeps {
            sessions,
            stores: Arc::new(ResourceStores::new(store_runtime(clock, cx))),
            columns: Arc::new(CoreColumns::new()),
            dispatcher: Rc::new(Ignore),
            exec: None,
            scale: None,
            set_image: None,
            drain: None,
            rollback: None,
            forward: None,
        };
        cx.new(|cx| DetailView::new(target, deps, Mount::Drawer, cx))
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |view, _, cx| {
        let detail = view.downcast::<DetailView>().expect("the root view");
        let now = at("2026-01-10T00:00:00Z");
        detail.update(cx, |detail, cx| {
            detail.pin_now(now, cx);
            detail.set_tab(DetailTab::Events, cx);
        });
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.capture_screenshot(window.into())
}

pub(crate) fn run() -> anyhow::Result<()> {
    check(
        "pods_table_warning_badges_dark",
        table(false)?,
        WIDTH,
        HEIGHT,
    )?;
    check(
        "pods_table_warning_badges_light",
        table(true)?,
        WIDTH,
        HEIGHT,
    )?;
    check(
        "detail_events_tab_dark",
        events_tab(false)?,
        TAB_WIDTH,
        TAB_HEIGHT,
    )?;
    check(
        "detail_events_tab_light",
        events_tab(true)?,
        TAB_WIDTH,
        TAB_HEIGHT,
    )
}
