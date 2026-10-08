//! Screenshots of the table's states (E07-S10), rendered through `Window::render_to_image`:
//! loading, empty, forbidden, unauthorized, error, and rows with the stale badge. The rows case
//! hides the Age and Restarts columns, which read the wall clock, so the golden does not rot.
//!
//! Each case scripts the fake cluster's watch (never lists, lists nothing, 403, 401, a failure
//! with a long message) and opens the real table over it. `harness = false`: on macOS the
//! platform text system can only be created on the process main thread. Needs a GPU device, so it
//! only builds with `--features screenshot` and runs in the nightly job:
//! `cargo test -p oxikube_resources_ui --features screenshot --test states_screenshot`.
//!
//! Regenerate the goldens with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::{path::Path, process::ExitCode, rc::Rc, sync::Arc, time::Duration};

use gpui::{App, AppContext as _, HeadlessAppContext, px, size};
use oxikube_app::store::ResourceStores;
use oxikube_app::{ClusterSessionManager, CoreColumns};
use oxikube_domain::OxiError;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_ports::{ClockPort, ClusterContext, Delta, DeltaBatch, SourceId, StatePort as _};
use oxikube_resources_ui::table::{
    ColumnPrefs, ResourceTable, ResourceTableDeps, prefs_key, store_runtime,
};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort, Timeline,
    headless::HEADLESS_SCALE_FACTOR,
    pod,
    screenshot::{RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path},
};
use oxikube_workspace::CommandDispatcher;

const WIDTH: f32 = 960.0;
const HEIGHT: f32 = 420.0;

struct Ignore;

impl CommandDispatcher for Ignore {
    fn dispatch(&self, _: Command, _: &mut App) {}
}

#[derive(Clone, Copy)]
enum Case {
    Loading,
    Empty,
    Forbidden,
    Unauthorized,
    Failed,
    StaleRows,
}

impl Case {
    const ALL: [Case; 6] = [
        Case::Loading,
        Case::Empty,
        Case::Forbidden,
        Case::Unauthorized,
        Case::Failed,
        Case::StaleRows,
    ];

    fn name(self) -> &'static str {
        match self {
            Case::Loading => "states_loading",
            Case::Empty => "states_empty",
            Case::Forbidden => "states_forbidden",
            Case::Unauthorized => "states_unauthorized",
            Case::Failed => "states_error",
            Case::StaleRows => "states_stale_rows",
        }
    }
}

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn deployments_kind() -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new("apps", "v1", "Deployment"),
        preferred: true,
        plural: "deployments".into(),
        singular: "deployment".into(),
        short_names: vec!["deploy".into()],
        categories: Vec::new(),
        verbs: VerbSet::from_names(["list", "watch"]),
        namespaced: true,
    }
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

fn fixture(
    case: Case,
) -> (
    ClusterSessionManager,
    ClusterId,
    Arc<FakeClockPort>,
    ResourceKind,
    impl FnOnce(),
) {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let ports = connector.ports_for(&cluster);
    let kind = match case {
        Case::Forbidden => deployments_kind(),
        _ => pods_kind(),
    };
    let watch = &ports.resources.script().watch;
    match case {
        Case::Loading => {
            watch.push_ok(Timeline::<DeltaBatch<oxikube_domain::Resource>>::new().keep_open());
        }
        Case::Empty => {}
        Case::Forbidden => {
            watch.push_err(OxiError::forbidden(
                "deployments.apps is forbidden: User \"developer\" cannot list resource \"deployments\" in API group \"apps\" at the cluster scope",
            ));
        }
        Case::Unauthorized => {
            watch.push_err(OxiError::auth(
                "the server has asked for the client to provide credentials (token expired)",
                false,
            ));
        }
        Case::Failed => {
            watch.push_err(OxiError::not_found(
                "the server could not find the requested resource\nrequest failed: GET /api/v1/pods?limit=500 returned 404 from the aggregated API",
            ));
        }
        Case::StaleRows => {
            let in_ns = |name: &str| pod().namespace("shop").name(name).node("kind-worker");
            let rows = vec![
                in_ns("api-7c9d").running().ip("10.244.0.12").build(),
                in_ns("cart-5f6b").running().ip("10.244.0.14").build(),
                in_ns("web-6a4f").running().ip("10.244.0.7").build(),
            ];
            watch.push_ok(
                Timeline::new()
                    .ok_at(
                        Duration::ZERO,
                        DeltaBatch::from_deltas(vec![Delta::Restarted(rows)]),
                    )
                    .err_at(
                        Duration::from_secs(1),
                        OxiError::network("connection reset by peer"),
                    ),
            );
        }
    }
    let clock = Arc::new(FakeClockPort::default());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster)).expect("connect");
    let resources = ports.resources.clone();
    let after = move || {
        if matches!(case, Case::StaleRows) {
            resources.clock().advance(Duration::from_secs(1));
        }
    };
    (sessions, cluster, clock, kind, after)
}

/// A saved layout without the Age and Restarts columns for the case that shows rows: the table
/// reads ages off the wall clock, so a picture with them changes every day and fails the nightly.
fn state_for(case: Case) -> Arc<FakeStatePort> {
    let state = Arc::new(FakeStatePort::new());
    if matches!(case, Case::StaleRows) {
        let layout = ColumnPrefs {
            visible: [("age".to_owned(), false), ("restarts".to_owned(), false)].into(),
            ..ColumnPrefs::default()
        };
        futures::executor::block_on(state.kv_set(
            &prefs_key(&pods_kind().gvk).expect("key"),
            serde_json::to_value(layout).expect("prefs json"),
        ))
        .expect("store prefs");
    }
    state
}

fn render(case: Case) -> anyhow::Result<RgbaImage> {
    let (sessions, cluster, clock, kind, after) = fixture(case);
    let state = state_for(case);
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
        cx.new(|cx| ResourceTable::new(cluster, kind, deps, window, cx))
    })?;
    cx.run_until_parked();
    after();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

/// The golden of the stale badge is the nightly runner's picture. The badge label ("Stale ·
/// reconnecting") shapes about a pixel narrower on the macOS of some developer machines, which
/// shifts the Retry button and the filter box beside it: 0.32 % of the pixels, so that case allows
/// 0.5 % instead of the default 0.1 %. A missing badge, row or column still fails by a wide margin.
fn tolerance(name: &str) -> Tolerance {
    match name {
        "states_stale_rows" => Tolerance {
            max_diff_ratio: 0.005,
            ..Tolerance::default()
        },
        _ => Tolerance::default(),
    }
}

fn check(name: &str, image: RgbaImage) -> anyhow::Result<()> {
    let scale = HEADLESS_SCALE_FACTOR;
    anyhow::ensure!(
        image.dimensions() == (WIDTH as u32 * scale, HEIGHT as u32 * scale),
        "unexpected image size {:?}",
        image.dimensions()
    );
    anyhow::ensure!(
        distinct_colors_at_least(&image, 6),
        "{name}: frame looks blank"
    );
    let goldens = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens");
    let golden = golden_path(&goldens, name);
    let updating =
        std::env::var_os("OXIKUBE_UPDATE_GOLDENS").is_some_and(|v| v != "0" && !v.is_empty());
    if golden.exists() || updating {
        check_golden(&image, &golden, tolerance(name))?;
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
    for case in Case::ALL {
        if let Err(err) = render(case).and_then(|image| check(case.name(), image)) {
            eprintln!("{} screenshot failed: {err:#}", case.name());
            return ExitCode::FAILURE;
        }
    }
    println!("resource table states screenshots: ok");
    ExitCode::SUCCESS
}
