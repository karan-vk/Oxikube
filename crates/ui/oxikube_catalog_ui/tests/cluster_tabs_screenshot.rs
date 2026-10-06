//! Cluster tabs and the hotbar, rendered through `Window::render_to_image` (E06-S04).
//!
//! - `cluster_tabs_prod`: two connected clusters with different colours (production red, staging
//!   blue) as two tabs next to the catalog's, the red one displayed, with a favourite that is not
//!   connected on the hotbar, and each cluster's own sidebar and pane.
//! - `cluster_tabs_staging`: the same window with the blue cluster displayed: its own sidebar and
//!   pane, its own colour on the hotbar bar and on the stripe.
//!
//! `harness = false`: on macOS the platform text system can only be created on the process main
//! thread, which libtest worker threads are not. Needs a GPU device (Metal, or Vulkan such as Mesa
//! lavapipe on Linux), so it only builds with `--features screenshot` and runs in the nightly job:
//! `cargo test -p oxikube_catalog_ui --features screenshot --test cluster_tabs_screenshot`.
//!
//! Checks: the PNG has the window's pixel size; the frame is not blank; the two colours are on
//! screen (the hotbar dots and the stripes); and, when a golden exists for this OS
//! (`tests/goldens/<os>/<name>.png`), the image matches it within tolerance. Regenerate with
//! `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_app::session::SessionOptions;
use oxikube_app::{ClusterCatalog, ClusterSessionManager};
use oxikube_catalog_ui::catalog::test_support::{
    RecordingDispatcher, cluster_id as id, context, source,
};
use oxikube_catalog_ui::{Hotbar, HotbarDeps};
use oxikube_domain::ClusterColour;
use oxikube_domain::command::Command;
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use oxikube_workspace::cluster_tab::{TabsDispatcher, cluster_hsla};
use oxikube_workspace::test_support::{TestItem, TestPanel};
use oxikube_workspace::{ClusterTabs, ClusterTabsDeps, CommandDispatcher, DockPosition, Workspace};

const WIDTH: f32 = 1100.0;
const HEIGHT: f32 = 520.0;
const RED: ClusterColour = ClusterColour::rgb(0xe5, 0x39, 0x35);
const BLUE: ClusterColour = ClusterColour::rgb(0x1e, 0x88, 0xe5);

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn render(displayed: &'static str) -> anyhow::Result<RgbaImage> {
    let names = ["prod-eu", "staging-eu", "dev-local"];
    let source = Arc::new(
        FakeClusterSourcePort::new()
            .with_sources([source()])
            .with_contexts(names.iter().map(|n| context(n))),
    );
    let state = Arc::new(FakeStatePort::new());
    let clock = Arc::new(FakeClockPort::default());
    let catalog = ClusterCatalog::new(source.clone(), state.clone(), clock.clone());
    let sessions =
        ClusterSessionManager::new(Arc::new(FakeClusterConnectorPort::new()), source, clock);
    for (name, colour) in [("prod-eu", RED), ("staging-eu", BLUE)] {
        sessions.open(
            &context(name),
            SessionOptions {
                colour: Some(colour),
                ..Default::default()
            },
        );
        futures::executor::block_on(sessions.connect(&id(name)))?;
    }
    // A favourite that is not connected: a muted tile with no state dot.
    futures::executor::block_on(catalog.set_favourite(&id("dev-local"), Some(true)))?;

    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);

        let workspace = cx.new(|cx| Workspace::new(window, cx));
        let home = TestItem::build("Clusters", cx);
        workspace.update(cx, |ws, cx| ws.open_item(home, window, cx));

        let recorder: Rc<dyn CommandDispatcher> = Rc::new(RecordingDispatcher::new());
        let deps = ClusterTabsDeps::new(sessions.clone(), state.clone(), recorder.clone())
            .with_setup(|tab, session, window, cx| {
                // Each cluster has its own sidebar and its own first view.
                let inner = tab.read(cx).workspace().clone();
                let sidebar = TestPanel::build(DockPosition::Left, "Sidebar", cx);
                let pods = TestItem::build(format!("Pods in {}", session.title()), cx);
                inner.update(cx, |ws, cx| {
                    ws.add_panel(sidebar, window, cx);
                    ws.open_item(pods, window, cx);
                });
            });
        let tabs = ClusterTabs::start(&workspace, deps, window, cx);
        tabs.update(cx, |tabs, cx| {
            tabs.apply(
                &Command::ClusterSelect {
                    cluster: id(displayed),
                },
                window,
                cx,
            )
        });
        let dispatcher: Rc<dyn CommandDispatcher> =
            Rc::new(TabsDispatcher::new(recorder, tabs.read(cx).command_sink()));
        let hotbar = cx.new(|cx| {
            Hotbar::new(
                HotbarDeps::new(
                    catalog.clone(),
                    sessions.clone(),
                    tabs,
                    dispatcher,
                    state.clone(),
                ),
                window,
                cx,
            )
        });
        workspace.update(cx, |ws, cx| ws.set_strip(Some(hotbar.into()), cx));
        oxikube_ui::root::new_root(workspace, window, cx)
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.capture_screenshot(window.into())
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
    for (cluster, colour) in [("prod-eu", RED), ("staging-eu", BLUE)] {
        anyhow::ensure!(
            has_colour(&image, cluster_hsla(colour)),
            "{name}: {cluster}'s colour is not on screen"
        );
    }

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
    check("cluster_tabs_prod", render("prod-eu")?)?;
    check("cluster_tabs_staging", render("staging-eu")?)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("cluster tabs screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("cluster tabs screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
