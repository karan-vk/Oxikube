//! The keymap editor page, rendered through `Window::render_to_image` (E21-S07).
//!
//! - `keymap_list`: the shipped default keymap plus a user file that rebinds one key, unbinds
//!   another with `null` and adds a binding; the unbound commands are hidden.
//! - `keymap_search`: the list narrowed by a keystroke typed the other way round
//!   (`Cmd+Shift+P`, the keymap's `cmd-shift-p`), showing the default marked "overridden" and the
//!   "unbound by you" row.
//! - `keymap_list_light`: the first frame in the light tokens.
//! - `keymap_recorder` (E21-S08): the key recorder on the user's `keymap::ShowEditor` binding
//!   with `ctrl-alt-k` recorded in `ResourceTable`, listing the binding it clashes with (the
//!   user's `keymap::OpenUser`, everywhere) and the Keep both / Replace them choice.
//!
//! `harness = false` and `--features screenshot`, like the metrics page's test. Checks the image
//! size, that it is not blank and, when a golden exists for this OS, that it matches within
//! tolerance. Regenerate with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a screenshot test reports on stdout and stderr"
)]

use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, HeadlessAppContext, Keystroke, px, size};
use oxikube_keymap::{KeymapOptions, KeymapPlatform};
use oxikube_settings_ui::keymap::recorder::KeybindRecorder;
use oxikube_settings_ui::keymap::{KeymapEditorDeps, KeymapEditorView};
use oxikube_settings_ui::metrics::test_support::RecordingDispatcher;
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};

const WIDTH: f32 = 1100.0;
const HEIGHT: f32 = 640.0;

const USER: &str = r#"[
  { "bindings": { "ctrl-alt-k": "keymap::OpenUser", "cmd-shift-p": null, "cmd-,": "keymap::ShowEditor" } }
]"#;

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn render(
    query: Option<&str>,
    show_unbound: bool,
    tokens: oxikube_ui::Tokens,
) -> anyhow::Result<RgbaImage> {
    let mut cx = headless();
    let mut page = None;
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, tokens);
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        oxikube_keymap::init_with_text(USER, KeymapOptions::default(), cx);
        let deps = KeymapEditorDeps {
            dispatcher: Rc::new(RecordingDispatcher::new()),
            workspace: None,
        };
        let view = cx.new(|cx| KeymapEditorView::new(deps, window, cx));
        page = Some(view.clone());
        oxikube_ui::root::new_root(view, window, cx)
    })?;
    if let Some(view) = page {
        cx.update_window(window.into(), |_, window, cx| {
            view.update(cx, |view, cx| {
                view.set_show_unbound(show_unbound, cx);
                if let Some(text) = query {
                    view.type_query(text, window, cx);
                }
            });
        })?;
    }
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

fn render_recorder(tokens: oxikube_ui::Tokens) -> anyhow::Result<RgbaImage> {
    let mut cx = headless();
    let mut held = None;
    let window = cx.open_window(size(px(560.), px(420.)), |window, cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, tokens);
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        oxikube_keymap::init_with_text(USER, KeymapOptions::default(), cx);
        let row = oxikube_keymap::keymap_rows(cx)
            .into_iter()
            .find(|row| row.action == "keymap::ShowEditor" && !row.keystrokes.is_empty())
            .expect("the user's binding of keymap::ShowEditor");
        let view = cx.new(|cx| {
            KeybindRecorder::new(
                &row,
                KeymapPlatform::MacOs,
                Rc::new(RecordingDispatcher::new()),
                window,
                cx,
            )
        });
        held = Some(view.clone());
        oxikube_ui::root::new_root(view, window, cx)
    })?;
    if let Some(view) = held {
        cx.update_window(window.into(), |_, window, cx| {
            view.update(cx, |recorder, cx| {
                recorder.type_context("ResourceTable", window, cx);
                let stroke = Keystroke::parse("ctrl-alt-k").expect("a keystroke");
                recorder.capture(&stroke, cx);
                recorder.stop(cx);
            });
        })?;
    }
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

fn check(name: &str, image: RgbaImage) -> anyhow::Result<()> {
    let scale = HEADLESS_SCALE_FACTOR;
    let expected = if name == "keymap_recorder" {
        (560 * scale, 420 * scale)
    } else {
        (WIDTH as u32 * scale, HEIGHT as u32 * scale)
    };
    anyhow::ensure!(
        image.dimensions() == expected,
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
    check(
        "keymap_list",
        render(None, false, oxikube_ui::Tokens::dark())?,
    )?;
    check(
        "keymap_list_light",
        render(None, false, oxikube_ui::Tokens::light())?,
    )?;
    check(
        "keymap_search",
        render(Some("Cmd+Shift+P"), true, oxikube_ui::Tokens::dark())?,
    )?;
    check(
        "keymap_recorder",
        render_recorder(oxikube_ui::Tokens::dark())?,
    )
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("keymap screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("keymap screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
