//! The guard's dry run in the confirmation dialog (E19-S07), in light and dark: a patch previewed
//! as the E10 diff of the live object against the server's result, with a server warning; and the
//! banner of an API that cannot dry-run the request.
//!
//! Same harness and gate as `screenshot.rs` (`harness = false`, needs a GPU device, feature
//! `screenshot`, nightly job): `cargo test -p oxikube_editor --features screenshot --test
//! dry_run_screenshot`. Goldens live in `tests/goldens/<os>/dry_run_<state>_{dark,light}.png`;
//! regenerate with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a screenshot test reports on stdout and stderr"
)]

use std::{path::Path, process::ExitCode, sync::Arc};

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_app::PreviewChange;
use oxikube_app::apply::Change;
use oxikube_domain::ClusterColour;
use oxikube_domain::audit::Initiator;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::safety::Risk;
use oxikube_testkit::{
    headless::HEADLESS_SCALE_FACTOR,
    screenshot::{RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path},
};
use oxikube_ui::{Tokens, root::new_root};
use oxikube_workspace::{ConfirmDialog, ConfirmPrompt, PreviewNote, Workspace};
use serde_json::{Value, json};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = 720.0;

fn deployment(replicas: u32, image: &str, version: &str) -> Value {
    json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "name": "web", "namespace": "shop", "resourceVersion": version,
            "labels": {"app": "web", "tier": "frontend"}
        },
        "spec": {
            "replicas": replicas,
            "selector": {"matchLabels": {"app": "web"}},
            "template": {
                "metadata": {"labels": {"app": "web"}},
                "spec": {"containers": [{"name": "web", "image": image, "ports": [{"containerPort": 8080}]}]}
            }
        },
        "status": {"replicas": 2}
    })
}

fn scenes() -> Vec<(&'static str, ConfirmPrompt)> {
    let prod = Some(ClusterColour::rgb(0xd0, 0x30, 0x30));
    let target = ResourceRef::namespaced(
        ClusterId::new("/kubeconfig", &ContextName::new("prod-eu")),
        Gvk::new("apps", "v1", "Deployment"),
        "shop",
        "web",
    );
    let change = PreviewChange {
        target,
        change: Change::Update,
        live: Some(Arc::new(deployment(2, "nginx:1.26", "7"))),
        result: Arc::new(deployment(3, "nginx:1.27", "8")),
    };
    let base = || {
        ConfirmPrompt::new("Apply?", "Apply Deployment shop/web on prod-eu.")
            .with_confirm_label("Apply")
            .with_cluster("prod-eu", prod)
            .with_initiator(Initiator::Agent)
            .with_risk(Some(Risk::Low))
    };
    let diff = base()
        .with_preview(Some(PreviewNote::Rehearsed(
            "Dry run: Deployment shop/web can be applied".into(),
        )))
        .with_changes(vec![change], 0)
        .with_warnings(vec![
            "unknown field \"spec.template.spec.containers[0].imagePullPolicyy\"".into(),
        ]);
    let no_preview = base().with_preview(Some(PreviewNote::Unavailable(
        "dryRun is not supported by the aggregated API metrics.example.com".into(),
    )));
    vec![("diff", diff), ("no_preview", no_preview)]
}

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn render(prompt: &ConfirmPrompt, tokens: Tokens) -> anyhow::Result<RgbaImage> {
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, tokens);
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        // Installs the diff view in the dialog.
        oxikube_editor::init(cx);
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        let prompt = prompt.clone();
        let dialog = cx.new(|cx| ConfirmDialog::new(prompt, |_, _, _| {}, |_| {}, window, cx));
        workspace.update(cx, |ws, cx| ws.show_modal(dialog.clone(), window, cx));
        dialog.update(cx, |dialog, cx| dialog.focus_initial(window, cx));
        new_root(workspace, window, cx)
    })?;
    // The diff is computed on the background executor.
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
    for (name, prompt) in scenes() {
        check(
            &format!("dry_run_{name}_dark"),
            render(&prompt, Tokens::dark())?,
        )?;
        check(
            &format!("dry_run_{name}_light"),
            render(&prompt, Tokens::light())?,
        )?;
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("dry run screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("dry run screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
