//! Screenshots of the resource views, rendered through `Window::render_to_image`.
//!
//! - `pods_table_tones`: running, pending, container-creating, crash-looping, failed, succeeded
//!   and terminating pods, one row selected; the Status cells in the theme's `oxikube` colours.
//! - `detail_deployment_dark`, `detail_deployment_light` (`detail`): the detail drawer of a
//!   Deployment in both themes: header with status chip, owner-less metadata, conditions, status.
//!
//! The Age and Restarts columns (ages, last-restart times) are hidden through a saved layout so
//! the picture does not change with the clock. `harness = false`: on macOS the platform text system can only be created on the
//! process main thread. Needs a GPU device (Metal, or Vulkan such as Mesa lavapipe on Linux), so
//! it only builds with `--features screenshot` and runs in the nightly job:
//! `cargo test -p oxikube_resources_ui --features screenshot --test screenshot`.
//!
//! Regenerate the golden with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::{path::Path, process::ExitCode, rc::Rc, sync::Arc};

use gpui::{App, AppContext as _, HeadlessAppContext, px, size};
use oxikube_app::store::ResourceStores;
use oxikube_app::{ClusterSessionManager, CoreColumns};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_ports::{ClockPort, ClusterContext, SourceId, StatePort as _};
use oxikube_resources_ui::table::{
    ColumnPrefs, ResourceTable, ResourceTableDeps, prefs_key, store_runtime,
};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
    headless::HEADLESS_SCALE_FACTOR,
    pod,
    screenshot::{RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path},
};
use oxikube_workspace::CommandDispatcher;

#[path = "screenshot/detail.rs"]
mod detail;

const WIDTH: f32 = 960.0;
const HEIGHT: f32 = 320.0;

struct Ignore;

impl CommandDispatcher for Ignore {
    fn dispatch(&self, _: Command, _: &mut App) {}
}

pub(crate) fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn pods_kind() -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new("", "v1", "Pod"),
        preferred: true,
        plural: "pods".into(),
        singular: "pod".into(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: VerbSet::from_names(["list", "watch"]),
        namespaced: true,
    }
}

/// A connected cluster serving one pod in each state, and a saved layout without Age.
fn fixture() -> (
    ClusterSessionManager,
    ClusterId,
    Arc<FakeStatePort>,
    Arc<FakeClockPort>,
) {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let ports = connector.ports_for(&cluster);
    let in_ns = |name: &str| pod().namespace("shop").name(name).node("kind-worker");
    for resource in [
        in_ns("api-7c9d")
            .running()
            .restarts(1)
            .ip("10.244.0.12")
            .build(),
        in_ns("cart-5f6b").pending().build(),
        in_ns("checkout-4d2a").container_creating().build(),
        in_ns("payments-9e1c")
            .crash_loop()
            .restarts(14)
            .ip("10.244.0.31")
            .build(),
        in_ns("reports-2b8f").failed().build(),
        in_ns("seed-db-1a3e").succeeded().build(),
        in_ns("web-6a4f").terminating().ip("10.244.0.7").build(),
    ] {
        ports.resources.insert(resource);
    }
    let clock = Arc::new(FakeClockPort::default());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster)).expect("connect");
    let state = Arc::new(FakeStatePort::new());
    let layout = ColumnPrefs {
        // Ages and last-restart times move with the clock.
        visible: [("age".to_owned(), false), ("restarts".to_owned(), false)].into(),
        ..ColumnPrefs::default()
    };
    futures::executor::block_on(state.kv_set(
        &prefs_key(&pods_kind().gvk).expect("key"),
        serde_json::to_value(layout).expect("prefs json"),
    ))
    .expect("store prefs");
    (sessions, cluster, state, clock)
}

fn render() -> anyhow::Result<RgbaImage> {
    let (sessions, cluster, state, clock) = fixture();
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        oxikube_runtime::init_deterministic(cx);
        cx.set_reduce_motion(true);
        let clock: Arc<dyn ClockPort> = clock;
        let deps = ResourceTableDeps {
            sessions,
            stores: Arc::new(ResourceStores::new(store_runtime(clock, cx))),
            columns: Arc::new(CoreColumns::new()),
            state,
            dispatcher: Rc::new(Ignore),
            actions: None,
        };
        cx.new(|cx| ResourceTable::new(cluster, pods_kind(), deps, window, cx))
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |view, _, cx| {
        let table = view.downcast::<ResourceTable>().expect("the root view");
        table.update(cx, |table, cx| {
            table.move_cursor(1, false, cx);
            table.move_cursor(3, false, cx);
        });
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

/// Checks `image` (a `width` x `height` window) against the golden `name`.
pub(crate) fn check(name: &str, image: RgbaImage, width: f32, height: f32) -> anyhow::Result<()> {
    let scale = HEADLESS_SCALE_FACTOR;
    anyhow::ensure!(
        image.dimensions() == (width as u32 * scale, height as u32 * scale),
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
    let results = [
        render().and_then(|image| check("pods_table_tones", image, WIDTH, HEIGHT)),
        detail::run(),
    ];
    let mut failed = false;
    for result in results {
        if let Err(err) = result {
            eprintln!("resource screenshot failed: {err:#}");
            failed = true;
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        println!("resource screenshots: ok");
        ExitCode::SUCCESS
    }
}
