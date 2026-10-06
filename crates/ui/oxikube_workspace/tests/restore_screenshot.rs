//! Session restore (E06-S11): the window right after a restore, rendered through
//! `Window::render_to_image`.
//!
//! - `restore_placeholders`: three restored cluster tabs next to the catalog's; the displayed one
//!   (`staging-eu`) has not connected yet, and its tab says so; `dev-local`'s tab waits for the
//!   first time it is shown.
//! - `restore_failed`: the displayed cluster (`prod-eu`) failed to connect: its own tab shows the
//!   error, and the others are placeholders, unaffected.
//!
//! Same harness and gate as `screenshot.rs` (`harness = false`, needs a GPU device, feature
//! `screenshot`, nightly job): `cargo test -p oxikube_workspace --features screenshot --test
//! restore_screenshot`. Goldens live in `tests/goldens/<os>/restore_*.png`; regenerate with
//! `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::{path::Path, process::ExitCode, rc::Rc, sync::Arc};

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_app::{
    ClusterSessionManager,
    session::{
        SessionOptions,
        restore::{RestorePlan, SavedTabs},
    },
};
use oxikube_domain::{
    ClusterColour, OxiError,
    command::Command,
    ids::{ClusterId, ContextName},
};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
    headless::HEADLESS_SCALE_FACTOR,
    screenshot::{RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path},
};
use oxikube_workspace::{
    ClusterTabs, ClusterTabsDeps, CommandDispatcher, Workspace, test_support::TestItem,
};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = 300.0;

/// Nothing is sent from a screenshot.
struct Ignore;

impl CommandDispatcher for Ignore {
    fn dispatch(&self, _: Command, _: &mut gpui::App) {}
}

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn context(name: &str) -> ClusterContext {
    let context = ContextName::new(name);
    ClusterContext::new(
        ClusterId::new("/home/me/.kube/config", &context),
        context,
        SourceId("kubeconfig".into()),
    )
}

/// The window after a restore of `prod-eu`, `staging-eu`, `dev-local` with `displayed` shown. When
/// `prod_fails`, `prod-eu` is the cluster the restore connected, and it failed.
fn render(displayed: &str, prod_fails: bool) -> anyhow::Result<RgbaImage> {
    let names = ["prod-eu", "staging-eu", "dev-local"];
    let contexts: Vec<_> = names.iter().map(|n| context(n)).collect();
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let sessions = ClusterSessionManager::new(
        connector.clone(),
        Arc::new(FakeClusterSourcePort::new().with_contexts(contexts.clone())),
        Arc::new(FakeClockPort::default()),
    );
    for (context, colour) in contexts.iter().zip([
        Some(ClusterColour::rgb(0xe5, 0x39, 0x35)),
        Some(ClusterColour::rgb(0x1e, 0x88, 0xe5)),
        None,
    ]) {
        sessions.open(
            context,
            SessionOptions {
                colour,
                ..Default::default()
            },
        );
    }
    if prod_fails {
        connector
            .connect_script_for(&contexts[0].cluster)
            .push_err(OxiError::unsupported(
                "the API server at prod-eu.example:6443 did not answer",
            ));
        futures::executor::block_on(sessions.connect(&contexts[0].cluster))?;
    }
    let plan = RestorePlan::resolve(
        Some(&SavedTabs::new(
            contexts.iter().map(|c| c.cluster.clone()).collect(),
            contexts
                .iter()
                .find(|c| c.context.as_str() == displayed)
                .map(|c| c.cluster.clone()),
        )),
        &contexts,
    );

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
        let deps = ClusterTabsDeps::new(
            sessions.clone(),
            Arc::new(FakeStatePort::new()),
            Rc::new(Ignore),
        );
        let tabs = ClusterTabs::start(&workspace, deps, window, cx);
        tabs.update(cx, |tabs, cx| tabs.hold_placeholders(&plan, window, cx));
        oxikube_ui::root::new_root(workspace, window, cx)
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
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
    check("restore_placeholders", render("staging-eu", false)?)?;
    check("restore_failed", render("prod-eu", true)?)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("restore screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("restore screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
