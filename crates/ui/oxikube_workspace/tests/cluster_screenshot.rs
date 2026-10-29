//! The production preset in light and dark (E06-S09), and the guard-mode status item in each
//! state plus five palette-coloured tabs (E19-S05): a cluster tab with its red dot and lock, the
//! status bar item "prod-eu, Read-only", and the toast a refused mutation shows.
//!
//! Same harness and gate as `screenshot.rs` (`harness = false`, needs a GPU device, feature
//! `screenshot`, nightly job): `cargo test -p oxikube_workspace --features screenshot --test
//! cluster_screenshot`. Goldens live in `tests/goldens/<os>/cluster_*.png`;
//! regenerate with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a screenshot test reports on stdout and stderr"
)]

use std::{path::Path, process::ExitCode, sync::Arc};

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_app::{ClusterSessionManager, session::SessionOptions};
use oxikube_domain::{ClusterPalette, ClusterPreset, ids::ContextName};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort,
    headless::HEADLESS_SCALE_FACTOR,
    screenshot::{RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path},
};
use oxikube_ui::{Tokens, root::new_root};
use oxikube_workspace::{
    ClusterMark, ClusterStatusItem, GuardStatusItem, StatusSide, Workspace, cluster::denial_toast,
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

/// What a frame shows beyond the tab and the left status item.
#[derive(Clone, Copy)]
enum Scene {
    /// The E06-S09 production frame: refused-mutation toast, no guard item.
    Prod,
    /// The right-hand guard item for a cluster that is (not) read-only.
    Guard { read_only: bool },
    /// One tab per palette colour (plus a plain one).
    Palette,
}

fn render(tokens: Tokens, scene: Scene) -> anyhow::Result<RgbaImage> {
    let context = ClusterContext::new(
        PROD.parse()?,
        ContextName::new("prod-eu"),
        SourceId("kubeconfig".into()),
    );
    let manager = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new().with_contexts([context.clone()])),
        Arc::new(FakeClockPort::default()),
    );
    // What the "Production" preset writes: red, read-only on.
    let read_only = match scene {
        Scene::Guard { read_only } => read_only,
        Scene::Prod | Scene::Palette => ClusterPreset::Prod.read_only().unwrap_or(false),
    };
    manager.open(
        &context,
        SessionOptions {
            read_only,
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
        let guard = cx.new(|cx| GuardStatusItem::new(manager.clone(), cx));
        guard.update(cx, |item, cx| {
            item.set_cluster(Some(context.cluster.clone()), cx)
        });
        let palette_tabs: Vec<_> = ClusterPalette::ALL
            .into_iter()
            .map(|p| {
                let mark = ClusterMark {
                    colour: Some(p.colour()),
                    read_only: p == ClusterPalette::Red,
                };
                cx.new(|cx| TestItem::new(p.name(), cx).with_cluster(mark))
            })
            .collect();
        let refused = denial_toast(&oxikube_app::DispatchError::ReadOnly {
            cluster: context.cluster.clone(),
            context: context.context.clone(),
        });
        workspace.update(cx, |ws, cx| {
            ws.register_status_item(StatusSide::Left, 0, status, cx);
            match scene {
                Scene::Prod => {
                    ws.show_toast(refused, cx);
                }
                Scene::Guard { .. } => {
                    ws.register_status_item(StatusSide::Right, 0, guard, cx);
                }
                Scene::Palette => {}
            }
            if matches!(scene, Scene::Palette) {
                for item in palette_tabs {
                    ws.open_item(item, window, cx);
                }
            } else {
                ws.open_item(plain, window, cx);
                ws.open_item(tab, window, cx);
            }
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
    for (suffix, tokens) in [("dark", Tokens::dark()), ("light", Tokens::light())] {
        check(
            &format!("cluster_prod_{suffix}"),
            render(tokens, Scene::Prod)?,
        )?;
        for (state, read_only) in [("guarded", false), ("readonly", true)] {
            check(
                &format!("cluster_guard_{state}_{suffix}"),
                render(tokens, Scene::Guard { read_only })?,
            )?;
        }
        check(
            &format!("cluster_palette_{suffix}"),
            render(tokens, Scene::Palette)?,
        )?;
    }
    Ok(())
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
