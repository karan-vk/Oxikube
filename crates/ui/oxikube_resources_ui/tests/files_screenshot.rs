//! Screenshots of the pod file browser (E20-S07): a directory listing, and the same listing with
//! the transfers panel showing a running download, a finished upload and a failed one. Rendered
//! through `Window::render_to_image` over a scripted fake cluster, so the toolbar, the path bar,
//! the table and the panel are the real view's output.
//!
//! `harness = false`: on macOS the platform text system can only be created on the process main
//! thread. Needs a GPU device, so it only builds with `--features screenshot` and runs in the
//! nightly job: `cargo test -p oxikube_resources_ui --features screenshot --test
//! files_screenshot`. Regenerate the goldens with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a screenshot test reports on stdout and stderr"
)]

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_app::ClusterSessionManager;
use oxikube_app::command_bus::{CommandBus, CommandRegistry};
use oxikube_app::files::{Direction, FileRunner, FileService, TransferStart, register_commands};
use oxikube_app::guard::MutationGuard;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_ports::{ClusterContext, SourceId, TransferProgress};
use oxikube_resources_ui::files::{FilesRequest, FilesView, FilesViewDeps};
use oxikube_testkit::{
    ExecScript, FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeFsPort,
    FakeStatePort,
    headless::HEADLESS_SCALE_FACTOR,
    pod,
    screenshot::{RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path},
};

const WIDTH: f32 = 1000.0;
const HEIGHT: f32 = 520.0;

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

const LISTING: &str = "total 48\n\
drwxr-xr-x 1 root root 4096 2024-03-05 12:34:56.000000000 +0000 .\n\
drwxr-xr-x 1 root root 4096 2024-03-05 12:34:56.000000000 +0000 ..\n\
drwxr-xr-x 2 root root 4096 2024-02-01 08:00:00.000000000 +0000 conf.d\n\
drwxr-xr-x 3 root root 4096 2024-02-01 08:00:00.000000000 +0000 ssl\n\
-rw-r--r-- 1 root root 1210 2024-03-05 12:34:56.000000000 +0000 nginx.conf\n\
-rw-r--r-- 1 root root 5349 2023-10-11 00:00:00.000000000 +0000 mime.types\n\
-rwxr-xr-x 1 root root 2411520 2024-01-15 22:59:59.000000000 +0000 server\n\
lrwxrwxrwx 1 root root 12 2024-03-05 12:30:00.000000000 +0000 current -> releases/12\n\
-rw------- 1 www-data www-data 33 2024-03-05 11:00:00.000000000 +0000 .htpasswd\n";

struct Scene {
    cx: HeadlessAppContext,
    window: gpui::AnyWindowHandle,
    service: Arc<FileService>,
}

fn scene() -> anyhow::Result<Scene> {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let ports = connector.ports_for(&cluster);
    ports
        .resources
        .insert(pod().namespace("shop").name("web-0").running().build());
    ports
        .resources
        .script()
        .get
        .push_ok(pod().namespace("shop").name("web-0").running().build());
    ports
        .exec_stream
        .script()
        .exec
        .push_ok(ExecScript::new().stdout(LISTING));
    let clock = Arc::new(FakeClockPort::default());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster))?;
    let service = Arc::new(FileService::new(
        sessions.clone(),
        Arc::new(FakeFsPort::new()),
    ));
    let mut registry = CommandRegistry::new();
    register_commands(&mut registry, service.clone())?;
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(sessions, Arc::new(FakeStatePort::new()), clock),
    );
    let runner = FileRunner::new(bus, "me", service.clone());
    let pod_ref = ResourceRef::namespaced(cluster, Gvk::new("", "v1", "Pod"), "shop", "web-0");

    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        oxikube_runtime::init_deterministic(cx);
        cx.set_reduce_motion(true);
        cx.new(|cx| {
            FilesView::new(
                FilesRequest {
                    pod: pod_ref,
                    container: None,
                    path: Some("/etc/nginx".into()),
                },
                FilesViewDeps { runner },
                None,
                window,
                cx,
            )
        })
    })?;
    cx.run_until_parked();
    cx.advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    Ok(Scene {
        cx,
        window: window.into(),
        service,
    })
}

fn capture(scene: &mut Scene) -> anyhow::Result<RgbaImage> {
    scene.cx.run_until_parked();
    scene.cx.advance_clock(Duration::from_millis(150));
    scene.cx.run_until_parked();
    scene
        .cx
        .update_window(scene.window, |_, window, cx| window.draw(cx).clear(cx))?;
    scene.cx.run_until_parked();
    scene.cx.capture_screenshot(scene.window)
}

fn check(name: &str, image: &RgbaImage) -> anyhow::Result<()> {
    let scale = HEADLESS_SCALE_FACTOR;
    anyhow::ensure!(
        image.dimensions() == (WIDTH as u32 * scale, HEIGHT as u32 * scale),
        "unexpected image size {:?}",
        image.dimensions()
    );
    anyhow::ensure!(
        distinct_colors_at_least(image, 8),
        "{name}: frame looks blank"
    );
    let goldens = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens");
    let golden = golden_path(&goldens, name);
    let updating =
        std::env::var_os("OXIKUBE_UPDATE_GOLDENS").is_some_and(|v| v != "0" && !v.is_empty());
    if golden.exists() || updating {
        check_golden(image, &golden, Tolerance::default())?;
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
    let mut scene = scene()?;
    let listing = capture(&mut scene)?;
    check("files_listing", &listing)?;

    // A running download at 38%, a finished upload and a failed download.
    let board = scene.service.board().clone();
    let place = "shop/web-0 (app)".to_owned();
    let running = board.begin(TransferStart {
        direction: Direction::Download,
        place: place.clone(),
        remote: "/var/log/app.log".into(),
        local: "app.log".into(),
        total: Some(11_534_336),
    });
    (running.progress)(TransferProgress {
        bytes: 4_383_000,
        total: Some(11_534_336),
    });
    let done = board.begin(TransferStart {
        direction: Direction::Upload,
        place: place.clone(),
        remote: "/tmp/notes.txt".into(),
        local: "notes.txt".into(),
        total: Some(2048),
    });
    (done.progress)(TransferProgress {
        bytes: 2048,
        total: Some(2048),
    });
    board.finish(done.id, Ok(None));
    let failed = board.begin(TransferStart {
        direction: Direction::Download,
        place,
        remote: "/etc/shadow".into(),
        local: "shadow".into(),
        total: None,
    });
    board.finish(
        failed.id,
        Err("cat could not access /etc/shadow in container app of pod shop/web-0: Permission denied".into()),
    );
    scene
        .cx
        .update_window(scene.window, |_, window, _| window.refresh())?;
    let panel = capture(&mut scene)?;
    check("files_transfers", &panel)?;
    Ok(())
}

fn main() -> ExitCode {
    if let Err(err) = run() {
        eprintln!("files screenshot failed: {err:#}");
        return ExitCode::FAILURE;
    }
    println!("files screenshot: ok");
    ExitCode::SUCCESS
}
