//! Headless screenshot mode (`--features screenshot`, dev and nightly CI only).
//!
//! `OXIKUBE_SCREENSHOT=out.png oxikube` renders the main window off-screen through
//! `Window::render_to_image` (via `oxikube_testkit::headless`), writes a PNG and exits: status 0
//! on success, 1 on any failure. No window is shown and nothing waits on wall-clock time; the
//! GPUI executor is driven deterministically until it is idle before the frame is captured.
//!
//! The window is the app's: the real init order ([`startup::init`]) over testkit fakes (the
//! embedded settings, an in-memory state db, a cluster source with [`CONTEXTS`] and no network)
//! and the real mount ([`crate::mount`]), so the picture is the catalog home with the hotbar and
//! the status bar, as a user sees it on launch.

use std::sync::Arc;
use std::{path::Path, process::ExitCode};

use anyhow::Context as _;
use gpui::{Pixels, Size, px, size};
use oxikube::startup::{self, StartupEnv};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, ClusterSource, SourceId, SourceKind};
use oxikube_testkit::{FakeClusterSourcePort, TestPorts, headless, screenshot};

/// The kubeconfig contexts the screenshot's catalog lists.
pub const CONTEXTS: [&str; 4] = ["kind-oxikube", "prod-eu", "staging-eu", "dev-local"];

/// Environment variable naming the PNG to write.
pub const ENV_VAR: &str = "OXIKUBE_SCREENSHOT";

/// Logical size of the captured window. The PNG is this times
/// [`headless::HEADLESS_SCALE_FACTOR`].
pub const WINDOW_SIZE: Size<Pixels> = size(px(1280.0), px(800.0));

/// Renders the main window and writes it to `path`. Returns the process exit code.
pub fn run(path: &Path) -> ExitCode {
    match capture_to(path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("oxikube: screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn capture_to(path: &Path) -> anyhow::Result<()> {
    let image = render()?;
    screenshot::save_png(&image, path)
}

/// Renders the main view into an image of `WINDOW_SIZE * HEADLESS_SCALE_FACTOR` pixels.
pub fn render() -> anyhow::Result<screenshot::RgbaImage> {
    let ports = ports();
    let mut cx = headless::headless_context_with_assets(Arc::new(oxikube_ui::Assets));
    cx.update(|cx| startup::init(cx, StartupEnv::test_with(&ports)))
        .context("the init order")?;
    // Pin the appearance: the theme follows the system, which differs between machines.
    cx.update(|cx| oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark()));
    let window = cx
        .open_window(WINDOW_SIZE, |window, cx| {
            oxikube_workspace::window::build_root_mounted(
                window,
                cx,
                None,
                oxikube::mount::mount_main_window,
                |content, _| content,
            )
        })
        .context("opening headless window")?;
    cx.run_until_parked();
    headless::capture_window(&mut cx, window.into())
}

/// The fakes: one kubeconfig file with [`CONTEXTS`].
fn ports() -> TestPorts {
    let source = SourceId("kubeconfig".to_owned());
    let contexts = CONTEXTS.map(|name| {
        let context = ContextName::new(name);
        ClusterContext {
            server: Some(format!("https://{name}.example:6443")),
            cluster_name: Some(name.to_owned()),
            user: Some(format!("{name}-admin")),
            ..ClusterContext::new(
                ClusterId::new("~/.kube/config", &context),
                context,
                source.clone(),
            )
        }
    });
    let clusters = FakeClusterSourcePort::new()
        .with_sources([ClusterSource {
            id: source,
            kind: SourceKind::KubeconfigFile,
            label: "~/.kube/config".to_owned(),
            path: None,
        }])
        .with_contexts(contexts);
    TestPorts {
        clusters: Arc::new(clusters),
        ..TestPorts::empty()
    }
}
