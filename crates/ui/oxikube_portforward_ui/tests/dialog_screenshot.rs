//! Screenshots of the port-forward start dialog (E15-S04): the dialog as a Pod's Forward button
//! opens it (ports offered, Auto, loopback), and after a start failed because the local port is
//! busy (message, suggestion and the "Use Auto" fix). Rendered through
//! `Window::render_to_image`, so the pixels are the real view's output.
//!
//! `harness = false`: on macOS the platform text system can only be created on the process main
//! thread. Needs a GPU device, so it only builds with `--features screenshot` and runs in the
//! nightly job: `cargo test -p oxikube_portforward_ui --features screenshot --test
//! dialog_screenshot`. Regenerate the goldens with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a screenshot test reports on stdout and stderr"
)]

use std::net::SocketAddr;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use gpui::{AppContext as _, HeadlessAppContext, WeakEntity, px, size};
use oxikube_app::command_bus::{CommandBus, CommandRegistry};
use oxikube_app::portforward::ObjectWatch;
use oxikube_app::store::{Spawner, StoreQuery};
use oxikube_app::{
    ClusterSessionManager, ForwardBackend, ForwardStarter, MutationGuard, PortForwardManager,
};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::portforward::ForwardPort;
use oxikube_portforward_ui::{PortChoice, StartDialog, StartForm, StartProblem};
use oxikube_ports::PortForwardError;
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};

const WIDTH: f32 = 640.0;
const HEIGHT: f32 = 700.0;

struct Backend;

impl ForwardBackend for Backend {
    fn port_forward(&self, _: &ClusterId) -> Option<Arc<dyn oxikube_ports::PortForwardPort>> {
        None
    }

    fn is_read_only(&self, _: &ClusterId) -> bool {
        false
    }

    fn watch(&self, _: &ClusterId, _: StoreQuery) -> Option<ObjectWatch> {
        None
    }
}

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn starter() -> ForwardStarter {
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
    );
    let guard = MutationGuard::new(sessions, Arc::new(FakeStatePort::new()), clock.clone());
    let bus = CommandBus::new(CommandRegistry::new(), guard);
    let spawner: Arc<dyn Spawner> = Arc::new(|_: futures::future::BoxFuture<'static, ()>| {});
    let manager = PortForwardManager::new(Arc::new(Backend), spawner, clock, None);
    ForwardStarter::new(bus, "alice", manager)
}

fn render(failed: bool) -> anyhow::Result<RgbaImage> {
    let cluster = ClusterId::new("/home/me/.kube/config", &ContextName::new("kind-oxikube"));
    let target = ResourceRef::namespaced(cluster, Gvk::new("", "v1", "Pod"), "shop", "api-6d8f");
    let ports = vec![
        PortChoice {
            number: 8080,
            name: Some("http".into()),
            container: Some("api".into()),
        },
        PortChoice {
            number: 9090,
            name: Some("metrics".into()),
            container: Some("api".into()),
        },
    ];
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        oxikube_runtime::init_deterministic(cx);
        cx.set_reduce_motion(true);
        let starter = starter();
        cx.new(|cx| {
            let mut dialog = StartDialog::new(
                StartForm::new(target, ports),
                Some(ForwardPort::Number(8080)),
                starter,
                WeakEntity::new_invalid(),
                window,
                cx,
            );
            if failed {
                let addr: SocketAddr = "127.0.0.1:8080".parse().expect("address");
                dialog.fill("8080", "8080", "127.0.0.1", window, cx);
                dialog.show_problem(
                    StartProblem::of_forward(&PortForwardError::BindFailed {
                        addr,
                        kind: std::io::ErrorKind::AddrInUse,
                    }),
                    cx,
                );
            }
            dialog
        })
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

fn main() -> ExitCode {
    for (name, failed) in [
        ("start_dialog_dark", false),
        ("start_dialog_error_dark", true),
    ] {
        if let Err(err) = render(failed).and_then(|image| check(name, image)) {
            eprintln!("{name} screenshot failed: {err:#}");
            return ExitCode::FAILURE;
        }
    }
    println!("start dialog screenshots: ok");
    ExitCode::SUCCESS
}
