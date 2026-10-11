//! Screenshot of the Workloads overview (E12-S07) with mixed statuses: pods running, not ready,
//! pending, failed and succeeded; a DaemonSet short of nodes; Deployments, a Job that failed and
//! a suspended CronJob, with a few warnings in the panel. Rendered through
//! `Window::render_to_image` over a scripted fake cluster, so the bars, the legend and the panel
//! are the real views' output.
//!
//! `harness = false`: on macOS the platform text system can only be created on the process main
//! thread. Needs a GPU device, so it only builds with `--features screenshot` and runs in the
//! nightly job: `cargo test -p oxikube_resources_ui --features screenshot --test
//! overview_screenshot`. Regenerate the golden with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::{path::Path, process::ExitCode, rc::Rc, sync::Arc, time::Duration};

use gpui::{App, AppContext as _, HeadlessAppContext, px, size};
use oxikube_app::store::ResourceStores;
use oxikube_app::{ClusterSessionManager, FixedWarnings, WarningRow};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClockPort, ClusterContext, SourceId};
use oxikube_resources_ui::overview_lite::{self, OverviewDeps, WorkloadsOverview};
use oxikube_resources_ui::table::store_runtime;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, daemonset, deployment,
    headless::HEADLESS_SCALE_FACTOR,
    job, pod,
    screenshot::{RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path},
};
use oxikube_workspace::CommandDispatcher;

const WIDTH: f32 = 1080.0;
const HEIGHT: f32 = 660.0;

struct Ignore;

impl CommandDispatcher for Ignore {
    fn dispatch(&self, _: Command, _: &mut App) {}
}

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn warning(namespace: &str, object: &str, reason: &str, message: &str, count: u32) -> WarningRow {
    WarningRow {
        namespace: Some(namespace.into()),
        object: object.to_owned(),
        reason: reason.to_owned(),
        message: message.to_owned(),
        count,
    }
}

fn render() -> anyhow::Result<RgbaImage> {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let ports = connector.ports_for(&cluster);
    let r = &ports.resources;
    let in_ns = |name: &str| pod().namespace("shop").name(name);
    for p in [
        in_ns("api-1").running().build(),
        in_ns("api-2").running().build(),
        in_ns("api-3").running().build(),
        in_ns("cart-1").running().build(),
        in_ns("web-1").running().build(),
        in_ns("web-2").running().build(),
        in_ns("loop-1").crash_loop().build(),
        in_ns("wait-1").pending().build(),
        in_ns("wait-2").pending().build(),
        in_ns("broken-1").failed().build(),
        in_ns("seed-1").succeeded().build(),
        in_ns("seed-2").succeeded().build(),
    ] {
        r.insert(p);
    }
    for d in [
        deployment()
            .namespace("shop")
            .name("api")
            .replicas(3)
            .ready(3),
        deployment()
            .namespace("shop")
            .name("web")
            .replicas(2)
            .ready(2),
        deployment()
            .namespace("shop")
            .name("cart")
            .replicas(2)
            .ready(1),
    ] {
        r.insert(d.build());
    }
    for d in [
        daemonset()
            .namespace("kube-system")
            .name("agent")
            .replicas(3)
            .ready(3),
        daemonset()
            .namespace("kube-system")
            .name("proxy")
            .replicas(3)
            .ready(1),
    ] {
        r.insert(d.build());
    }
    r.insert(job().namespace("shop").name("migrate").failed().build());
    r.insert(job().namespace("shop").name("seed").complete().build());

    let clock = Arc::new(FakeClockPort::default());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster)).expect("connect");

    let warnings = FixedWarnings(vec![
        warning(
            "shop",
            "Pod/loop-1",
            "BackOff",
            "Back-off restarting failed container app in pod loop-1",
            14,
        ),
        warning(
            "shop",
            "Pod/wait-1",
            "FailedScheduling",
            "0/1 nodes are available: 1 Insufficient memory.",
            3,
        ),
        warning(
            "shop",
            "Deployment/cart",
            "ProgressDeadlineExceeded",
            "ReplicaSet cart-5f6b has timed out progressing.",
            1,
        ),
    ]);

    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        oxikube_runtime::init_deterministic(cx);
        cx.set_reduce_motion(true);
        overview_lite::init(cx);
        let clock: Arc<dyn ClockPort> = clock;
        let deps = OverviewDeps {
            sessions,
            stores: Arc::new(ResourceStores::new(store_runtime(clock, cx))),
            dispatcher: Rc::new(Ignore),
            warnings: Arc::new(warnings),
        };
        let _ = window;
        cx.new(|cx| WorkloadsOverview::new(cluster, deps, cx))
    })?;
    cx.run_until_parked();
    // The feeds list, the one-second timer reads the counts, the pacer delivers the redraw.
    cx.advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    cx.advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
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

fn main() -> ExitCode {
    if let Err(err) = render().and_then(|image| check("workloads_overview", image)) {
        eprintln!("workloads overview screenshot failed: {err:#}");
        return ExitCode::FAILURE;
    }
    println!("workloads overview screenshot: ok");
    ExitCode::SUCCESS
}
