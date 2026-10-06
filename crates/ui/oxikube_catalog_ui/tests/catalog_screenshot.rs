//! Catalog screenshots, rendered through `Window::render_to_image` (E06-S03).
//!
//! - `catalog_populated`: eight contexts from two kubeconfig files with favourites at the top,
//!   one connected, one failing, one needing credentials, one that cannot work as written, and
//!   last-used times.
//! - `catalog_searching`: the same window with `eu` typed into the search field.
//! - `catalog_empty`: no contexts, so the empty state explaining how to add kubeconfigs.
//!
//! `harness = false`: on macOS the platform text system can only be created on the process main
//! thread, which libtest worker threads are not. Needs a GPU device (Metal, or Vulkan such as Mesa
//! lavapipe on Linux), so it only builds with `--features screenshot` and runs in the nightly job:
//! `cargo test -p oxikube_catalog_ui --features screenshot --test catalog_screenshot`.
//!
//! Checks: the PNG has the window's pixel size; the frame is not blank; and, when a golden exists
//! for this OS (`tests/goldens/<os>/<name>.png`), the image matches it within tolerance.
//! Regenerate with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::cell::RefCell;
use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use jiff::Timestamp;
use oxikube_app::catalog::CATALOG_TABLE;
use oxikube_app::{ClusterCatalog, ClusterSessionManager};
use oxikube_catalog_ui::catalog::test_support::{
    RecordingDispatcher, cluster_id as id, context, source,
};
use oxikube_catalog_ui::{CatalogDeps, CatalogView};
use oxikube_domain::OxiError;
use oxikube_ports::{
    ClockPort as _, ClusterContext, SourceId, StateKey, StatePort as _, StateTable,
};
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use serde_json::json;

const WIDTH: f32 = 1100.0;
const HEIGHT: f32 = 520.0;

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

/// Eight contexts: five in the default file, three in a team file.
fn contexts() -> Vec<ClusterContext> {
    let mut all: Vec<ClusterContext> = [
        "prod-eu",
        "prod-us",
        "staging-eu",
        "staging-us",
        "dev-local",
        "qa-eu",
        "team-a-sandbox",
        "team-a-broken",
    ]
    .iter()
    .map(|name| context(name))
    .collect();
    for ctx in &mut all[5..] {
        ctx.source = SourceId("team".into());
    }
    all[7].problem = Some("user \"team-a\" is not defined in the kubeconfig".into());
    all
}

fn render(contexts: Vec<ClusterContext>, search: Option<&str>) -> anyhow::Result<RgbaImage> {
    let mut team = source();
    team.id = SourceId("team".into());
    team.label = "~/.kube/team-a.yaml".into();
    let has_contexts = !contexts.is_empty();
    let source = Arc::new(
        FakeClusterSourcePort::new()
            .with_sources([source(), team])
            .with_contexts(contexts),
    );
    let state = Arc::new(FakeStatePort::new());
    let clock = Arc::new(FakeClockPort::default());
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let catalog = ClusterCatalog::new(source.clone(), state.clone(), clock.clone());
    let sessions = ClusterSessionManager::new(connector.clone(), source, clock.clone());

    // Marks: two favourites and a few last-used times, relative to the fake clock.
    let table = StateTable::new(CATALOG_TABLE)?;
    let now: Timestamp = clock.now();
    for (name, favourite, minutes) in [
        ("prod-eu", true, Some(3)),
        ("staging-eu", true, None),
        ("prod-us", false, Some(95)),
        ("dev-local", false, Some(60 * 30)),
        ("qa-eu", false, Some(60 * 24 * 9)),
    ] {
        let mut row = json!({ "favourite": favourite });
        if let Some(minutes) = minutes {
            row["last_used"] = json!(now - Duration::from_secs(minutes * 60));
        }
        futures::executor::block_on(state.table_put(
            &table,
            &StateKey::new(id(name).as_str())?,
            row,
        ))?;
    }
    // States: one connected, one failing, one waiting for credentials.
    connector
        .connect_script_for(&id("staging-us"))
        .push_err(OxiError::internal(
            "dial tcp 10.0.3.7:6443: connect: connection refused",
        ));
    connector
        .connect_script_for(&id("qa-eu"))
        .push_err(OxiError::auth("the exec plugin needs a new token", false));
    if has_contexts {
        for name in ["prod-eu", "staging-us", "qa-eu"] {
            futures::executor::block_on(sessions.connect(&id(name)))?;
        }
    }

    let mut cx = headless();
    let view_slot = Rc::new(RefCell::new(None));
    let slot = view_slot.clone();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        let deps = CatalogDeps {
            catalog,
            sessions,
            dispatcher: Rc::new(RecordingDispatcher::new()),
            clock,
        };
        let view = cx.new(|cx| CatalogView::new(deps, window, cx));
        *slot.borrow_mut() = Some(view.clone());
        oxikube_ui::root::new_root(view, window, cx)
    })?;
    cx.run_until_parked();
    if let Some(search) = search {
        let view = view_slot.borrow().clone().expect("the window was built");
        cx.update_window(window.into(), |_, window, cx| {
            view.update(cx, |view, cx| view.set_search(search, window, cx));
        })?;
        cx.run_until_parked();
    }
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
    check("catalog_populated", render(contexts(), None)?)?;
    check("catalog_searching", render(contexts(), Some("eu"))?)?;
    check("catalog_empty", render(Vec::new(), None)?)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("catalog screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("catalog screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
