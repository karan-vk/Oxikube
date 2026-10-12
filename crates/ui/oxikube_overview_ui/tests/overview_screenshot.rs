//! The cluster overview, rendered through `Window::render_to_image` (E13-S09): the three modes the
//! story names, each in the dark and the light theme.
//!
//! - `overview_metrics_server_{dark,light}`: tiles, capacity bars with usage, top lists, info,
//!   warnings; no graphs.
//! - `overview_prometheus_{dark,light}`: the same with the last hour's graphs and sparklines.
//! - `overview_unavailable_{dark,light}`: no metrics-server and no Prometheus: the reasons, the
//!   tiles and allocation bars that need no metrics.
//!
//! `harness = false` and `--features screenshot`, like the other screenshot tests. Checks the
//! image size, that it is not blank and, when a golden exists for this OS, that it matches within
//! tolerance. Regenerate with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use futures::future::BoxFuture;
use gpui::{App, AppContext as _, HeadlessAppContext, px, size};
use oxikube_app::store::StoreRuntime;
use oxikube_app::{ClusterSessionManager, NoRecentWarnings, ResourceStores};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_overview_ui::test_support::{
    Preview, metrics_server_only, metrics_unavailable, with_prometheus,
};
use oxikube_overview_ui::{ClusterOverview, OverviewDeps};
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};
use oxikube_testkit::{FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort};
use oxikube_workspace::CommandDispatcher;

const WIDTH: f32 = 1280.0;
const HEIGHT: f32 = 860.0;

struct Nowhere;

impl CommandDispatcher for Nowhere {
    fn dispatch(&self, _: Command, _: &mut App) {}
}

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn deps() -> OverviewDeps {
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new()),
        Arc::new(FakeClockPort::default()),
    );
    let runtime = StoreRuntime {
        spawner: Arc::new(|_: BoxFuture<'static, ()>| {}),
        clock: Arc::new(FakeClockPort::default()),
        probe: None,
    };
    OverviewDeps {
        sessions,
        stores: Arc::new(ResourceStores::new(runtime)),
        dispatcher: Rc::new(Nowhere),
        metrics: None,
        history: None,
        warnings: Arc::new(NoRecentWarnings),
    }
}

fn render(preview: Preview, dark: bool) -> anyhow::Result<RgbaImage> {
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), move |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        let tokens = if dark {
            oxikube_ui::Tokens::dark()
        } else {
            oxikube_ui::Tokens::light()
        };
        oxikube_ui::set_tokens(cx, tokens);
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        let cluster = ClusterId::new("kubeconfig", &ContextName::new("prod"));
        let view = cx.new(|cx| ClusterOverview::preview(cluster, deps(), preview, cx));
        oxikube_ui::root::new_root(view, window, cx)
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
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

fn run() -> anyhow::Result<()> {
    for (dark, theme) in [(true, "dark"), (false, "light")] {
        check(
            &format!("overview_metrics_server_{theme}"),
            render(metrics_server_only(), dark)?,
        )?;
        check(
            &format!("overview_prometheus_{theme}"),
            render(with_prometheus(), dark)?,
        )?;
        check(
            &format!("overview_unavailable_{theme}"),
            render(metrics_unavailable(), dark)?,
        )?;
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("overview screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("overview screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
