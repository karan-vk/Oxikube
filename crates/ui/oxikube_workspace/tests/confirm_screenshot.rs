//! The confirmation dialog (E19-S06) in light and dark: the simple tier, the type-the-name tier
//! empty, partly typed and matched, and a bulk request listing its objects.
//!
//! Same harness and gate as `screenshot.rs` (`harness = false`, needs a GPU device, feature
//! `screenshot`, nightly job): `cargo test -p oxikube_workspace --features screenshot --test
//! confirm_screenshot`. Goldens live in `tests/goldens/<os>/confirm_<state>_{dark,light}.png`;
//! regenerate with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a screenshot test reports on stdout and stderr"
)]

use std::{path::Path, process::ExitCode, sync::Arc};

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_domain::{ClusterColour, audit::Initiator, safety::Risk};
use oxikube_testkit::{
    headless::HEADLESS_SCALE_FACTOR,
    screenshot::{RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path},
};
use oxikube_ui::{Tokens, root::new_root};
use oxikube_workspace::{ConfirmDialog, ConfirmItem, ConfirmPrompt, PreviewNote, Workspace};

const WIDTH: f32 = 640.0;
const HEIGHT: f32 = 640.0;

/// One picture: the prompt and what has been typed into its field.
struct Scene {
    name: &'static str,
    prompt: ConfirmPrompt,
    typed: &'static str,
}

fn scenes() -> Vec<Scene> {
    let prod = Some(ClusterColour::rgb(0xd0, 0x30, 0x30));
    let simple = ConfirmPrompt::new("Restart the workload?", "Restart deployment default/web.")
        .with_confirm_label("Restart")
        .with_cluster("prod-eu", prod)
        .with_risk(Some(Risk::Medium));
    let namespace = ConfirmPrompt::new("Delete the namespace?", "Delete namespace kube-system.")
        .with_confirm_label("Delete")
        .with_destructive(true)
        .with_cluster("prod-eu", prod)
        .with_risk(Some(Risk::Irreversible))
        .with_preview(Some(PreviewNote::Rehearsed(
            "Dry run: namespace kube-system can be deleted".into(),
        )))
        .with_typed_name("kube-system");
    let items: Vec<ConfirmItem> = (0..500)
        .map(|ix| {
            let risk = if ix % 50 == 0 {
                Risk::High
            } else {
                Risk::Medium
            };
            ConfirmItem::new(format!("Pod default/web-{ix}"), risk)
        })
        .collect();
    let bulk = ConfirmPrompt::new("Delete 500 objects?", "Delete 500 objects on prod-eu.")
        .with_confirm_label("Delete")
        .with_destructive(true)
        .with_cluster("prod-eu", prod)
        .with_initiator(Initiator::Agent)
        .with_risk(Some(Risk::High))
        .with_preview(Some(PreviewNote::Unavailable(
            "dry run not supported".into(),
        )))
        .with_typed_name("prod-eu")
        .with_items(items);
    vec![
        Scene {
            name: "simple",
            prompt: simple,
            typed: "",
        },
        Scene {
            name: "typed_empty",
            prompt: namespace.clone(),
            typed: "",
        },
        Scene {
            name: "typed_partial",
            prompt: namespace.clone(),
            typed: "kube-",
        },
        Scene {
            name: "typed_matched",
            prompt: namespace,
            typed: "kube-system",
        },
        Scene {
            name: "bulk",
            prompt: bulk,
            typed: "prod",
        },
    ]
}

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn render(scene: &Scene, tokens: Tokens) -> anyhow::Result<RgbaImage> {
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, tokens);
        cx.set_reduce_motion(true);
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        let prompt = scene.prompt.clone();
        let dialog = cx.new(|cx| ConfirmDialog::new(prompt, |_, _, _| {}, |_| {}, window, cx));
        workspace.update(cx, |ws, cx| ws.show_modal(dialog.clone(), window, cx));
        dialog.update(cx, |dialog, cx| {
            dialog.type_text(scene.typed, window, cx);
            dialog.focus_initial(window, cx);
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
    for scene in scenes() {
        check(
            &format!("confirm_{}_dark", scene.name),
            render(&scene, Tokens::dark())?,
        )?;
        check(
            &format!("confirm_{}_light", scene.name),
            render(&scene, Tokens::light())?,
        )?;
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("confirm screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("confirm screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
