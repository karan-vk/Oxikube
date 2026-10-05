//! Workspace screenshots, rendered through `Window::render_to_image`.
//!
//! - `workspace` (E05-S04, E05-S10): a left and a bottom dock with panels, the centre split in two
//!   panes of items with tab bars, the status bar with a left and a right item, and two toasts.
//! - `workspace_modal` (E05-S10): the same window with a confirmation dialog in the modal layer.
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

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_testkit::{
    headless::HEADLESS_SCALE_FACTOR,
    screenshot::{RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path},
};
use oxikube_ui::root::new_root;
use oxikube_workspace::{
    DialogModal, DockPosition, OpenOptions, SplitDirection, StatusSide, Toast, ToastAction,
    Workspace,
    test_support::{TestItem, TestPanel, TestStatusItem},
};

const WIDTH: f32 = 960.0;
const HEIGHT: f32 = 600.0;

fn render(with_modal: bool) -> anyhow::Result<RgbaImage> {
    let text_system = gpui_platform::current_platform(true).text_system();
    let mut cx =
        HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
            gpui_platform::current_headless_renderer()
        });
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        // A fade in flight would make the picture depend on the clock.
        oxikube_workspace::motion::set_reduce_motion(cx, true);
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

fn check(name: &str, with_modal: bool) -> anyhow::Result<()> {
    let image = render(with_modal)?;
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
    check("workspace", false)?;
    check("workspace_modal", true)
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
