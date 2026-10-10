//! Screenshots of the help overlay (E11-S10), rendered through the headless renderer:
//!
//! - `help_overlay_dark`, `help_overlay_light`: the keys of a pod table grouped under Resource,
//!   Pod, View and Navigation, with a user rebind (`User` chip), a vim base key (`Base` chip) and
//!   a default the user unbound (`Unbound by you`), the first entry selected.
//! - `help_overlay_search_dark`: the same list filtered by `log`, headers regrouped.
//!
//! `harness = false`: on macOS the platform text system can only be created on the process main
//! thread. Needs a GPU device (Metal, or Vulkan such as Mesa lavapipe on Linux), so it only builds
//! with `--features screenshot` and runs in the nightly job:
//! `cargo test -p oxikube_palette --features screenshot --test help_screenshot`.
//!
//! Regenerate the goldens with `OXIKUBE_UPDATE_GOLDENS=1`.

use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Result;
use gpui::{AppContext as _, Entity, px, size};
use oxikube_keymap::{ActiveBinding, BindingInfo, KeymapLayer, SuppressedBinding};
use oxikube_palette::help::{HelpModel, HelpOverlay, HelpScope};
use oxikube_testkit::gpui_test::{GoldenCase, ScreenshotApp, run_golden_cases};
use oxikube_testkit::screenshot::RgbaImage;
use oxikube_ui::root::Root;

const SIZE: (u32, u32) = (680, 600);

fn binding(action: &'static str, keys: &str, layer: KeymapLayer) -> ActiveBinding {
    ActiveBinding {
        action,
        binding: BindingInfo {
            keystrokes: keys.split_whitespace().map(str::to_owned).collect(),
            context: Some("ResourceTable && !Editing".to_owned()),
            layer: Some(layer),
        },
    }
}

fn model() -> HelpModel {
    use KeymapLayer::{Default, User, Vim};
    let active = vec![
        binding("resource_table::ViewYaml", "x", User),
        binding("resource_table::EditSelected", "e", Default),
        binding("resource_table::DeleteSelected", "ctrl-d", Default),
        binding("resource_table::OpenSelected", "enter", Default),
        binding("resource_table::CopyName", "cmd-c", Default),
        binding("resource_table::ViewLogs", "l", Default),
        binding("resource_table::ShellSelected", "s", Default),
        binding("resource_table::PortForward", "shift-f", Default),
        binding("help::Show", "?", Default),
        binding("palette::Toggle", "cmd-shift-p", Default),
        binding("table::SelectNext", "j", Vim),
        binding("table::SelectPrevious", "k", Vim),
        binding("resource_table::SelectFirst", "home", Default),
    ];
    let unbound = vec![SuppressedBinding {
        action: "resource_table::ViewDescribe",
        binding: BindingInfo {
            keystrokes: vec!["d".to_owned()],
            context: Some("ResourceTable && !Editing".to_owned()),
            layer: Some(Default),
        },
        by: User,
    }];
    HelpModel::build(
        HelpScope::Focused {
            innermost: "ResourceTable".into(),
        },
        active,
        unbound,
    )
}

fn render(query: &str, light: bool) -> Result<RgbaImage> {
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
    let mut overlay: Option<Entity<HelpOverlay>> = None;
    let window = app.open_window(size(px(SIZE.0 as f32), px(SIZE.1 as f32)), |window, cx| {
        let entity = cx.new(|cx| HelpOverlay::new(Arc::new(model()), window, cx));
        overlay = Some(entity.clone());
        cx.new(|cx| Root::new(entity, window, cx))
    })?;
    let overlay = overlay.expect("built");
    app.update(|cx| {
        window.update(cx, |_, window, cx| {
            let picker = overlay.read(cx).picker().clone();
            picker.update(cx, |picker, cx| picker.set_query(query, window, cx));
        })
    })?;
    app.capture(window)
}

fn dark() -> Result<RgbaImage> {
    render("", false)
}

fn light() -> Result<RgbaImage> {
    render("", true)
}

fn search_dark() -> Result<RgbaImage> {
    render("log", false)
}

fn main() -> ExitCode {
    run_golden_cases(
        env!("CARGO_MANIFEST_DIR"),
        &[
            GoldenCase {
                name: "help_overlay_dark",
                size: SIZE,
                render: dark,
            },
            GoldenCase {
                name: "help_overlay_light",
                size: SIZE,
                render: light,
            },
            GoldenCase {
                name: "help_overlay_search_dark",
                size: SIZE,
                render: search_dark,
            },
        ],
    )
}
