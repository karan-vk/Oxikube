//! The events view, rendered through `Window::render_to_image` (E14-S03), over a Warning-heavy
//! cluster: twelve events (eight warnings: back-offs, failed scheduling, OOM kills, a failed
//! probe), newest first, the second row selected.
//!
//! - `events_dark`, `events_light`: the view in each appearance.
//! - `events_faults`: the same with the faults toggle on (only the warnings), dark.
//!
//! `harness = false` and `--features screenshot`, like the other screenshot tests (see
//! `oxikube_catalog_ui/tests/catalog_screenshot.rs` for why). Checks the image size, that it is not
//! blank and, when a golden exists for this OS, that it matches within tolerance. Regenerate with
//! `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use jiff::{SignedDuration, Timestamp};
use oxikube_app::ClusterSessionManager;
use oxikube_app::events::{EventService, EventsConfig};
use oxikube_app::store::Spawner;
use oxikube_domain::command::Command;
use oxikube_domain::event::{Event, EventType};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_events_ui::{EventsView, EventsViewDeps};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeEventFeed, FakeFsPort,
    FakeStatePort, event,
};
use oxikube_workspace::CommandDispatcher;

const WIDTH: f32 = 1280.0;
const HEIGHT: f32 = 640.0;

struct Ignore;

impl CommandDispatcher for Ignore {
    fn dispatch(&self, _: Command, _: &mut gpui::App) {}
}

fn now() -> Timestamp {
    "2026-05-04T12:00:00Z".parse().unwrap()
}

/// `(type, reason, namespace, kind/name, source, count, minutes ago, message)`.
const EVENTS: [(EventType, &str, &str, &str, &str, u32, i64, &str); 12] = [
    (
        EventType::Warning,
        "BackOff",
        "shop",
        "Pod/cart-7d9f6-x2k4p",
        "kubelet",
        14,
        1,
        "Back-off restarting failed container cart in pod cart-7d9f6-x2k4p_shop",
    ),
    (
        EventType::Warning,
        "FailedScheduling",
        "shop",
        "Pod/search-0",
        "default-scheduler",
        3,
        2,
        "0/4 nodes are available: 4 Insufficient memory. preemption: 0/4 nodes are available",
    ),
    (
        EventType::Normal,
        "Pulled",
        "web",
        "Pod/web-5c8b7-q9zt2",
        "kubelet",
        1,
        3,
        "Successfully pulled image \"nginx:1.27\" in 1.204s",
    ),
    (
        EventType::Warning,
        "OOMKilled",
        "batch",
        "Pod/report-28843-ml4hc",
        "kubelet",
        2,
        4,
        "Container report exceeded its memory limit of 512Mi",
    ),
    (
        EventType::Warning,
        "Unhealthy",
        "web",
        "Pod/api-6f5d4-7jw2n",
        "kubelet",
        37,
        5,
        "Readiness probe failed: HTTP probe failed with statuscode: 503",
    ),
    (
        EventType::Normal,
        "ScalingReplicaSet",
        "web",
        "Deployment/api",
        "deployment-controller",
        1,
        6,
        "Scaled up replica set api-6f5d4 to 3",
    ),
    (
        EventType::Warning,
        "FailedMount",
        "data",
        "Pod/pg-1",
        "kubelet",
        6,
        7,
        "MountVolume.SetUp failed for volume \"pgdata\": secret \"pg-tls\" not found",
    ),
    (
        EventType::Normal,
        "Started",
        "kube-system",
        "Pod/coredns-7db6d-tm2x9",
        "kubelet",
        1,
        9,
        "Started container coredns",
    ),
    (
        EventType::Warning,
        "BackOff",
        "shop",
        "Pod/checkout-0",
        "kubelet",
        9,
        11,
        "Back-off pulling image \"registry.local/checkout:2.4.1\"",
    ),
    (
        EventType::Warning,
        "NodeNotReady",
        "default",
        "Node/worker-3",
        "node-controller",
        1,
        14,
        "Node worker-3 status is now: NodeNotReady",
    ),
    (
        EventType::Normal,
        "Scheduled",
        "web",
        "Pod/web-5c8b7-q9zt2",
        "default-scheduler",
        1,
        16,
        "Successfully assigned web/web-5c8b7-q9zt2 to worker-2",
    ),
    (
        EventType::Warning,
        "Evicted",
        "batch",
        "Pod/etl-28843-zz8kq",
        "kubelet",
        1,
        21,
        "The node was low on resource: ephemeral-storage.",
    ),
];

fn events() -> Vec<Event> {
    EVENTS
        .iter()
        .enumerate()
        .map(
            |(i, (kind, reason, ns, object, source, count, ago, message))| {
                let (kind_name, name) = object.split_once('/').unwrap();
                let scoped = (kind_name != "Node").then_some(*ns);
                let mut e = event()
                    .uid(&format!("e{i}"))
                    .reason(reason)
                    .message(message)
                    .regarding(kind_name, scoped, name, &format!("uid-{name}"))
                    .count(*count)
                    .build();
                e.event_type = *kind;
                e.namespace = Some((*ns).into());
                e.reporting_component = Some((*source).into());
                let seen = now() - SignedDuration::from_mins(*ago);
                e.last_seen = Some(seen);
                e.first_seen = Some(seen - SignedDuration::from_mins(i64::from(*count) * 3));
                e
            },
        )
        .collect()
}

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn render(tokens: oxikube_ui::Tokens, faults: bool) -> anyhow::Result<RgbaImage> {
    let cluster = ClusterId::new("/shots/kubeconfig", &ContextName::new("shots"));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let port = connector.ports_for(&cluster).events;
    port.install(FakeEventFeed::new().initial(events()));
    let source = Arc::new(
        FakeClusterSourcePort::new().with_contexts([ClusterContext::new(
            cluster.clone(),
            ContextName::new("shots"),
            SourceId("k".into()),
        )]),
    );
    let clock = Arc::new(FakeClockPort::new(now()));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster))?;

    let mut cx = headless();
    let service = EventService::new(sessions.clone(), clock.clone(), EventsConfig::default());
    let spawner: Arc<dyn Spawner> = cx.update(|cx| {
        let executor = cx.background_executor().clone();
        Arc::new(move |task| executor.spawn(task).detach()) as Arc<dyn Spawner>
    });
    service.start(spawner);
    let deps = EventsViewDeps {
        service,
        sessions,
        dispatcher: Rc::new(Ignore),
        state: Arc::new(FakeStatePort::new()),
        fs: Arc::new(FakeFsPort::new()),
        clock,
    };
    let mut view = None;
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, tokens);
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        let events = cx.new(|cx| EventsView::new(cluster, deps, None, window, cx));
        view = Some(events.clone());
        oxikube_ui::root::new_root(events, window, cx)
    })?;
    cx.run_until_parked();
    let view = view.expect("the view");
    view.update(&mut cx, |v, cx| {
        if faults {
            v.toggle_faults(cx);
        }
    });
    cx.run_until_parked();
    view.update(&mut cx, |v, cx| v.select(1, cx));
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

fn check(name: &str, image: RgbaImage) -> anyhow::Result<()> {
    let scale = HEADLESS_SCALE_FACTOR;
    anyhow::ensure!(
        image.dimensions() == (WIDTH as u32 * scale, HEIGHT as u32 * scale),
        "unexpected image size {:?}",
        image.dimensions()
    );
    anyhow::ensure!(
        distinct_colors_at_least(&image, 8),
        "{name}: frame looks blank"
    );
    let goldens = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens");
    let golden = golden_path(&goldens, name);
    let updating =
        std::env::var_os("OXIKUBE_UPDATE_GOLDENS").is_some_and(|v| v != "0" && !v.is_empty());
    if golden.exists() || updating {
        check_golden(&image, &golden, Tolerance::default())?;
        println!("{name} matches {}", golden.display());
    } else {
        println!(
            "no golden for {} yet; structural checks only",
            std::env::consts::OS
        );
    }
    Ok(())
}

fn run() -> anyhow::Result<()> {
    check("events_dark", render(oxikube_ui::Tokens::dark(), false)?)?;
    check("events_light", render(oxikube_ui::Tokens::light(), false)?)?;
    check("events_faults", render(oxikube_ui::Tokens::dark(), true)?)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("events screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("events screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
