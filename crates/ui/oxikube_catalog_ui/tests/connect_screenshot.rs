//! The connect lifecycle screens, rendered through `Window::render_to_image` (E06-S06): one
//! screenshot per state of a cluster tab.
//!
//! - `connect_connecting`: spinner (still, reduce-motion is on), the API server and context, Cancel.
//! - `connect_auth_required`: the plugin's message, the exec policy (forbid), how to sign in,
//!   a disabled Open terminal (no terminal yet) and Retry.
//! - `connect_error`: summary, expanded details, Copy details, Edit kubeconfig sources, Retry.
//! - `connect_degraded`: the banner above the cluster's own content.
//!
//! `harness = false`: on macOS the platform text system can only be created on the process main
//! thread, which libtest worker threads are not. Needs a GPU device (Metal, or Vulkan such as Mesa
//! lavapipe on Linux), so it only builds with `--features screenshot` and runs in the nightly job:
//! `cargo test -p oxikube_catalog_ui --features screenshot --test connect_screenshot`.
//!
//! Checks: the PNG has the window's pixel size; the frame is not blank; the state's accent colour
//! is on screen; and, when a golden exists for this OS (`tests/goldens/<os>/<name>.png`), the image
//! matches it within tolerance. Regenerate with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use futures::FutureExt as _;
use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_app::ClusterSessionManager;
use oxikube_catalog_ui::catalog::test_support::{
    RecordingDispatcher, cluster_id as id, context, source,
};
use oxikube_catalog_ui::connect::{ConnectDeps, ConnectView, tab_setup};
use oxikube_domain::OxiError;
use oxikube_ports::HealthSignal;
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use oxikube_workspace::{ClusterTabs, ClusterTabsDeps, CommandDispatcher, Workspace};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = 560.0;
const CLUSTER: &str = "prod-eu";

/// The state to put the cluster in.
#[derive(Clone, Copy)]
enum Scene {
    Connecting,
    AuthRequired,
    Error,
    Degraded,
}

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn render(scene: Scene) -> anyhow::Result<RgbaImage> {
    let source = Arc::new(
        FakeClusterSourcePort::new()
            .with_sources([source()])
            .with_contexts([context(CLUSTER)]),
    );
    let state = Arc::new(FakeStatePort::new());
    let clock = Arc::new(FakeClockPort::default());
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let sessions = ClusterSessionManager::new(connector.clone(), source, clock);
    let cluster = id(CLUSTER);

    // Drive the session into the scene's state before the window opens: the tab opens on it.
    let mut waiting = None;
    match scene {
        Scene::Connecting => {
            connector.hold();
            let manager = sessions.clone();
            let target = cluster.clone();
            let mut connect = async move { manager.connect(&target).await }.boxed();
            anyhow::ensure!((&mut connect).now_or_never().is_none(), "the attempt waits");
            waiting = Some(connect);
        }
        Scene::AuthRequired => {
            connector
                .connect_script_for(&cluster)
                .push_err(OxiError::auth(
                    "exec plugin `aws eks get-token` failed: the SSO session has expired, \
                 run `aws sso login --profile prod` and try again",
                    false,
                ));
            futures::executor::block_on(sessions.connect(&cluster))?;
        }
        Scene::Error => {
            connector
                .connect_script_for(&cluster)
                .push_err(OxiError::validation(
                    "dial tcp 10.20.30.40:6443: i/o timeout\nthe API server did not answer within \
                 30s (is the VPN up?)\ncontext: prod-eu",
                ));
            futures::executor::block_on(sessions.connect(&cluster))?;
        }
        Scene::Degraded => {
            futures::executor::block_on(sessions.connect(&cluster))?;
            connector.report(&cluster, HealthSignal::Unhealthy);
        }
    }

    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);

        let workspace = cx.new(|cx| Workspace::new(window, cx));
        let dispatcher: Rc<dyn CommandDispatcher> = Rc::new(RecordingDispatcher::new());
        let connect = ConnectDeps::new(sessions.clone(), dispatcher.clone()).with_sources(|_| {});
        let deps = ClusterTabsDeps::new(sessions.clone(), state.clone(), dispatcher)
            .with_setup(tab_setup(connect));
        let tabs = ClusterTabs::start(&workspace, deps, window, cx);
        if matches!(scene, Scene::Error) {
            // The error's details, open.
            let view = tabs.read(cx).tab(&cluster).and_then(|tab| {
                let ui = tab.read(cx).connect_ui()?;
                ui.body.clone().downcast::<ConnectView>().ok()
            });
            if let Some(view) = view {
                view.update(cx, |view, cx| view.toggle_details(cx));
            }
        }
        oxikube_ui::root::new_root(workspace, window, cx)
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    let image = cx.capture_screenshot(window.into())?;
    drop(waiting);
    Ok(image)
}

/// Whether some pixel of `image` is within a few levels of `colour`.
fn has_colour(image: &RgbaImage, colour: gpui::Hsla) -> bool {
    let rgba = gpui::Rgba::from(colour);
    let target = [
        (rgba.r * 255.).round() as i32,
        (rgba.g * 255.).round() as i32,
        (rgba.b * 255.).round() as i32,
    ];
    image.pixels().any(|pixel| {
        (0..3).all(|channel| (i32::from(pixel.0[channel]) - target[channel]).abs() <= 3)
    })
}

fn check(name: &str, accent: gpui::Hsla, image: RgbaImage) -> anyhow::Result<()> {
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
    anyhow::ensure!(
        has_colour(&image, accent),
        "{name}: the state's accent colour is not on screen"
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
    let colors = oxikube_ui::Tokens::dark().colors;
    check(
        "connect_connecting",
        colors.accent,
        render(Scene::Connecting)?,
    )?;
    check(
        "connect_auth_required",
        colors.warning,
        render(Scene::AuthRequired)?,
    )?;
    check("connect_error", colors.error, render(Scene::Error)?)?;
    check("connect_degraded", colors.warning, render(Scene::Degraded)?)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("connect screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("connect screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
