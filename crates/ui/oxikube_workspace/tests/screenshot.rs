//! Workspace screenshots, rendered through `Window::render_to_image`.
//!
//! - `workspace` (E05-S04, E05-S10): a left and a bottom dock with panels, the centre split in two
//!   panes of items with tab bars, the status bar with a left and a right item, and two toasts.
//! - `workspace_modal` (E05-S10): the same window with a confirmation dialog in the modal layer.
//! - `main_window_restoring` (E05-S13): the main window behind the startup placeholder, its saved
//!   layout still being read ("Restoring layout…" in the title bar over the default layout).
//!
//! `harness = false`: on macOS the platform text system can only be created on the process main
//! thread, which libtest worker threads are not. Needs a GPU device (Metal, or Vulkan such as Mesa
//! lavapipe on Linux), so it only builds with `--features screenshot` and runs in the nightly job:
//! `cargo test -p oxikube_workspace --features screenshot --test screenshot`.
//!
//! Checks: the PNG has the window's pixel size; the frame is not blank; and, when a golden exists
//! for this OS (`tests/goldens/<os>/workspace.png`), the image matches it within tolerance.
//! Regenerate with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::{path::Path, process::ExitCode, sync::Arc};

use async_trait::async_trait;
use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_domain::{OxiResult, audit::AuditRecord};
use oxikube_ports::{AuditQuery, StateKey, StatePort, StateTable};
use oxikube_testkit::{
    headless::HEADLESS_SCALE_FACTOR,
    screenshot::{RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path},
};
use oxikube_ui::root::new_root;
use oxikube_workspace::{
    DialogModal, DockPosition, OpenOptions, SplitDirection, StatusSide, Toast, ToastAction,
    Workspace,
    persistence::{LayoutStore, MAIN_WINDOW_ID},
    test_support::{TestItem, TestPanel, TestStatusItem},
    window::build_root_with_layout,
};
use serde_json::Value;

const WIDTH: f32 = 960.0;
const HEIGHT: f32 = 600.0;

/// A state db whose reads never finish: the restore stays in flight for the picture.
struct NeverReady;

#[async_trait]
impl StatePort for NeverReady {
    async fn kv_get(&self, _: &StateKey) -> OxiResult<Option<Value>> {
        std::future::pending().await
    }
    async fn kv_set(&self, _: &StateKey, _: Value) -> OxiResult<()> {
        std::future::pending().await
    }
    async fn kv_delete(&self, _: &StateKey) -> OxiResult<bool> {
        std::future::pending().await
    }
    async fn kv_list(&self, _: &str) -> OxiResult<Vec<(StateKey, Value)>> {
        std::future::pending().await
    }
    async fn table_get(&self, _: &StateTable, _: &StateKey) -> OxiResult<Option<Value>> {
        std::future::pending().await
    }
    async fn table_put(&self, _: &StateTable, _: &StateKey, _: Value) -> OxiResult<()> {
        std::future::pending().await
    }
    async fn table_delete(&self, _: &StateTable, _: &StateKey) -> OxiResult<bool> {
        std::future::pending().await
    }
    async fn table_list(
        &self,
        _: &StateTable,
        _: Option<usize>,
    ) -> OxiResult<Vec<(StateKey, Value)>> {
        std::future::pending().await
    }
    async fn append_audit(&self, _: &[AuditRecord]) -> OxiResult<()> {
        std::future::pending().await
    }
    async fn query_audit(&self, _: &AuditQuery) -> OxiResult<Vec<AuditRecord>> {
        std::future::pending().await
    }
}

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

/// The main window as the app opens it, while its saved layout is still being read.
fn render_restoring() -> anyhow::Result<RgbaImage> {
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        cx.set_reduce_motion(true);
        let store = LayoutStore::new(Arc::new(NeverReady), MAIN_WINDOW_ID)
            .expect("the main window's key is valid");
        build_root_with_layout(window, cx, Some(store), |content, _| content)
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

fn render(with_modal: bool) -> anyhow::Result<RgbaImage> {
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        // A fade in flight would make the picture depend on the clock.
        cx.set_reduce_motion(true);
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        let left = TestPanel::build(DockPosition::Left, "Clusters", cx);
        let bottom = TestPanel::build(DockPosition::Bottom, "Logs", cx);
        let pods = cx.new(|cx| TestItem::new("Pods", cx));
        let deployments = cx.new(|cx| TestItem::new("Deployments", cx));
        let yaml = cx.new(|cx| {
            let mut item = TestItem::new("web-0.yaml", cx);
            item.dirty = true;
            item
        });
        let cluster = cx.new(|_| TestStatusItem::new("kind-oxikube"));
        let read_only = cx.new(|_| TestStatusItem::new("read-only"));
        workspace.update(cx, |ws, cx| {
            ws.register_status_item(StatusSide::Left, 0, cluster, cx);
            ws.register_status_item(StatusSide::Right, 0, read_only, cx);
            ws.show_toast(
                Toast::error("Could not reach kind-oxikube")
                    .title("Connection failed")
                    .action(ToastAction::new("Retry", |_, _| {})),
                cx,
            );
            ws.show_toast(Toast::success("Copied pod name"), cx);
            if with_modal {
                let dialog = cx.new(|cx| {
                    DialogModal::new("Delete pod web-0?", cx)
                        .message("The pod is recreated by its ReplicaSet.")
                        .confirm_label("Delete")
                        .destructive()
                });
                ws.show_modal(dialog, window, cx);
            }
            ws.add_panel(left, window, cx);
            ws.add_panel(bottom, window, cx);
            ws.open_item(pods, window, cx);
            ws.open_item(deployments, window, cx);
            let yaml = ws.open_item_with(Box::new(yaml), OpenOptions::default(), window, cx);
            ws.move_item_to_split(yaml, SplitDirection::Right, window, cx);
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
    check("workspace", render(false)?)?;
    check("workspace_modal", render(true)?)?;
    check("main_window_restoring", render_restoring()?)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("workspace screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("workspace screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
