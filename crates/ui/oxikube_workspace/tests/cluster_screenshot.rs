//! The production preset in light and dark (E06-S09): a cluster tab with its red dot and lock, the
//! status bar item "prod-eu, Read-only", and the toast a refused mutation shows.
//!
//! Same harness and gate as `screenshot.rs` (`harness = false`, needs a GPU device, feature
//! `screenshot`, nightly job): `cargo test -p oxikube_workspace --features screenshot --test
//! cluster_screenshot`. Goldens live in `tests/goldens/<os>/cluster_prod_{dark,light}.png`;
//! regenerate with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::{path::Path, process::ExitCode, sync::Arc};

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_app::{ClusterSessionManager, session::SessionOptions};
use oxikube_domain::{ClusterPreset, ids::ContextName};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort,
    headless::HEADLESS_SCALE_FACTOR,
    screenshot::{RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path},
};
use oxikube_ui::{Tokens, root::new_root};
use oxikube_workspace::{
    ClusterMark, ClusterStatusItem, StatusSide, Workspace, cluster::denial_toast,
    test_support::TestItem,
};

const WIDTH: f32 = 720.0;
const HEIGHT: f32 = 240.0;
const PROD: &str = "3f2a9c1b7d4e8a60";

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn render(tokens: Tokens) -> anyhow::Result<RgbaImage> {
    let context = ClusterContext {
        cluster: PROD.parse()?,
        context: ContextName::new("prod-eu"),
        source: SourceId("kubeconfig".into()),
        server: None,
        default_namespace: None,
    };
    let manager = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new().with_contexts([context.clone()])),
        Arc::new(FakeClockPort::default()),
    );
    // What the "Production" preset writes: red, read-only on.
    manager.open(
        &context,
        SessionOptions {
            read_only: ClusterPreset::Prod.read_only().unwrap_or(false),
            colour: ClusterPreset::Prod.colour(),
            ..SessionOptions::default()
        },
    );
    let session = manager.get(&context.cluster).expect("opened above");
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, tokens);
        cx.set_reduce_motion(true);
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        let tab = cx.new(|cx| TestItem::new("prod-eu", cx).with_cluster(ClusterMark::of(&session)));
        let plain = cx.new(|cx| TestItem::new("lab", cx));
        let status = cx.new(|cx| ClusterStatusItem::new(manager.clone(), cx));
        status.update(cx, |item, cx| {
            item.set_cluster(Some(context.cluster.clone()), cx)
        });
        let refused = denial_toast(&oxikube_app::DispatchError::ReadOnly {
            cluster: context.cluster.clone(),
            context: context.context.clone(),
        });
        workspace.update(cx, |ws, cx| {
            ws.register_status_item(StatusSide::Left, 0, status, cx);
            ws.show_toast(refused, cx);
            ws.open_item(plain, window, cx);
            ws.open_item(tab, window, cx);
        });
        new_root(workspace, window, cx)
    })?;
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
    check("cluster_prod_dark", render(Tokens::dark())?)?;
    check("cluster_prod_light", render(Tokens::light())?)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("cluster screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("cluster screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
