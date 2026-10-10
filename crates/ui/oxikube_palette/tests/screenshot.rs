//! Screenshots of the picker (E11-S02), rendered through the headless renderer:
//!
//! - `picker_selected_dark`, `picker_selected_light`: twelve resource-like entries filtered by
//!   `core`, the matched characters highlighted and the second match selected, in both themes.
//! - `picker_empty_dark`: a query that matches nothing, with the "No matches" text.
//! - `command_palette_dark`: the command palette (E11-S03) over every declared command (eleven
//!   categories), on a pod table: key bindings, the recent commands first.
//! - `command_palette_show_all_light`: the same on a read-only cluster with "Show all" on, the
//!   commands that cannot run listed dimmed with the reason, filtered by `delete`.
//!
//! `harness = false`: on macOS the platform text system can only be created on the process main
//! thread. Needs a GPU device (Metal, or Vulkan such as Mesa lavapipe on Linux), so it only builds
//! with `--features screenshot` and runs in the nightly job:
//! `cargo test -p oxikube_palette --features screenshot --test screenshot`.
//!
//! Regenerate the goldens with `OXIKUBE_UPDATE_GOLDENS=1`.

use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Result;
use gpui::{AppContext as _, Entity, px, size};
use oxikube_app::{
    CommandContext, CommandIndex, CommandInfo, CommandTarget, MemoryRecents, RecentsStore,
    Selection,
};
use oxikube_domain::Capabilities;
use oxikube_domain::command::{self, CommandId, ViewContext};
use oxikube_domain::ids::Gvk;
use oxikube_palette::command_palette::{Outbox, PaletteParts, Snapshot};
use oxikube_palette::picker::test_support::TestDelegate;
use oxikube_palette::{CommandPalette, Picker};
use oxikube_testkit::gpui_test::{GoldenCase, ScreenshotApp, run_golden_cases};
use oxikube_testkit::screenshot::RgbaImage;
use oxikube_ui::root::Root;

const SIZE: (u32, u32) = (600, 260);

const ENTRIES: [&str; 12] = [
    "pod/kube-system/coredns-7db6d8ff4d-2xk9q",
    "pod/kube-system/coredns-7db6d8ff4d-wq8tz",
    "pod/kube-system/etcd-kind-control-plane",
    "pod/kube-system/kube-apiserver-kind-control-plane",
    "pod/kube-system/kube-proxy-5hcnm",
    "deployment/kube-system/coredns",
    "configmap/kube-system/coredns",
    "service/kube-system/kube-dns",
    "pod/shop/cart-5f6b",
    "pod/shop/checkout-4d2a",
    "pod/shop/web-6a4f",
    "namespace/shop",
];

/// The picker over [`ENTRIES`] with `query` typed and match `select` selected.
fn render(query: &str, select: usize, light: bool) -> Result<RgbaImage> {
    let mut app = ScreenshotApp::with_assets(Arc::new(oxikube_ui::Assets));
    app.update(|cx| {
        oxikube_ui::init(cx);
        let tokens = if light {
            oxikube_ui::Tokens::light()
        } else {
            oxikube_ui::Tokens::dark()
        };
        oxikube_ui::set_tokens(cx, tokens);
        cx.set_reduce_motion(true);
    });
    let mut picker: Option<Entity<Picker<TestDelegate>>> = None;
    let window = app.open_window(size(px(SIZE.0 as f32), px(SIZE.1 as f32)), |window, cx| {
        let entity = cx.new(|cx| Picker::uniform_list(TestDelegate::new(ENTRIES), window, cx));
        picker = Some(entity.clone());
        cx.new(|cx| Root::new(entity, window, cx))
    })?;
    let picker = picker.expect("built");
    app.update(|cx| {
        window.update(cx, |_, window, cx| {
            picker.update(cx, |picker, cx| picker.set_query(query, window, cx));
        })
    })?;
    app.update(|cx| {
        window.update(cx, |_, window, cx| {
            picker.update(cx, |picker, cx| {
                picker.set_selected_index(select, None, true, window, cx);
            });
        })
    })?;
    app.capture(window)
}

fn selected_dark() -> Result<RgbaImage> {
    render("core", 1, false)
}

fn selected_light() -> Result<RgbaImage> {
    render("core", 1, true)
}

fn empty_dark() -> Result<RgbaImage> {
    render("zzz", 0, false)
}

const PALETTE_SIZE: (u32, u32) = (700, 520);

/// The command palette over every declared command, for a table with a pod selected.
fn render_palette(query: &str, read_only: bool, show_all: bool, light: bool) -> Result<RgbaImage> {
    let mut app = ScreenshotApp::with_assets(Arc::new(oxikube_ui::Assets));
    app.update(|cx| {
        oxikube_ui::init(cx);
        let tokens = if light {
            oxikube_ui::Tokens::light()
        } else {
            oxikube_ui::Tokens::dark()
        };
        oxikube_ui::set_tokens(cx, tokens);
        cx.set_reduce_motion(true);
        // The shipped keymap: the rows show its bindings.
        oxikube_keymap::init_with_text("", oxikube_keymap::KeymapOptions::default(), cx);
    });
    let index = CommandIndex::new(
        command::COMMANDS
            .iter()
            .map(|meta| CommandInfo::new(meta, "screenshot", true)),
    )?;
    let mut context = CommandContext::new(ViewContext::Table)
        .with_capabilities(Capabilities::all())
        .selecting(Selection::one(Gvk::new("", "v1", "Pod")))
        .read_only(read_only);
    context.cluster_active = true;
    let recents = Arc::new(MemoryRecents::new());
    recents.record(CommandId::POD_VIEW_LOGS);
    recents.record(CommandId::NAMESPACE_SELECT);
    let snapshot = Snapshot::take(&index, &context);
    let categories: std::collections::BTreeSet<_> = snapshot
        .rows
        .iter()
        .map(|row| row.info.category())
        .collect();
    anyhow::ensure!(categories.len() >= 10, "{} categories", categories.len());
    let mut palette: Option<Entity<CommandPalette>> = None;
    let window = app.open_window(
        size(px(PALETTE_SIZE.0 as f32), px(PALETTE_SIZE.1 as f32)),
        |window, cx| {
            let parts = PaletteParts {
                snapshot,
                target: CommandTarget::none(),
                outbox: Outbox::default(),
                recents: recents as Arc<dyn RecentsStore>,
                workspace: gpui::WeakEntity::new_invalid(),
            };
            let entity = cx.new(|cx| CommandPalette::new(parts, window, cx));
            palette = Some(entity.clone());
            cx.new(|cx| Root::new(entity, window, cx))
        },
    )?;
    let palette = palette.expect("built");
    app.update(|cx| {
        window.update(cx, |_, window, cx| {
            palette.update(cx, |palette, cx| {
                if show_all {
                    palette.toggle_show_all(window, cx);
                }
                palette
                    .picker()
                    .update(cx, |picker, cx| picker.set_query(query, window, cx));
            });
        })
    })?;
    app.capture(window)
}

fn palette_dark() -> Result<RgbaImage> {
    render_palette("", false, false, false)
}

fn palette_show_all_light() -> Result<RgbaImage> {
    render_palette("delete", true, true, true)
}

fn main() -> ExitCode {
    run_golden_cases(
        env!("CARGO_MANIFEST_DIR"),
        &[
            GoldenCase {
                name: "picker_selected_dark",
                size: SIZE,
                render: selected_dark,
            },
            GoldenCase {
                name: "picker_selected_light",
                size: SIZE,
                render: selected_light,
            },
            GoldenCase {
                name: "picker_empty_dark",
                size: SIZE,
                render: empty_dark,
            },
            GoldenCase {
                name: "command_palette_dark",
                size: PALETTE_SIZE,
                render: palette_dark,
            },
            GoldenCase {
                name: "command_palette_show_all_light",
                size: PALETTE_SIZE,
                render: palette_show_all_light,
            },
        ],
    )
}
