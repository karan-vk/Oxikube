//! Screenshots of the namespace selector, rendered through `Window::render_to_image`.
//!
//! - `namespace_selector_open`: the dropdown with a selection of two namespaces, two favourites
//!   with their digits, and the rest of the list.
//! - `namespace_selector_restricted`: a cluster that refuses to list namespaces (403): the
//!   explanation and the typed names.
//!
//! `harness = false`: on macOS the platform text system can only be created on the process main
//! thread, which libtest worker threads are not. Needs a GPU device (Metal, or Vulkan such as Mesa
//! lavapipe on Linux), so it only builds with `--features screenshot` and runs in the nightly job:
//! `cargo test -p oxikube_catalog_ui --features screenshot --test screenshot`.
//!
//! Regenerate the goldens with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::{path::Path, process::ExitCode, sync::Arc};

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_app::ClusterSessionManager;
use oxikube_app::session::namespaces::{NamespacePrefs, NamespaceService, prefs_key};
use oxikube_catalog_ui::namespaces::NamespaceSelector;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::{OxiError, Resource};
use oxikube_ports::{ClusterContext, SourceId, StatePort as _};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
    headless::HEADLESS_SCALE_FACTOR,
    screenshot::{RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path},
};
use serde_json::json;

const WIDTH: f32 = 360.0;
const HEIGHT: f32 = 400.0;

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn namespace(name: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Namespace", "metadata": { "name": name },
    }))
    .expect("namespace json")
}

/// A connected cluster with `namespaces` (or a forbidden list), `prefs` remembered.
fn service(
    namespaces: &[&str],
    forbidden: bool,
    prefs: &NamespacePrefs,
) -> (NamespaceService, ClusterId) {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext {
        cluster: cluster.clone(),
        context,
        source: SourceId("kubeconfig".into()),
        server: None,
        default_namespace: None,
        cluster_name: None,
        user: None,
        problem: None,
    };
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let ports = connector.ports_for(&cluster);
    for name in namespaces {
        ports.resources.insert(namespace(name));
    }
    if forbidden {
        for _ in 0..3 {
            ports
                .resources
                .script()
                .list_metadata
                .push_err(OxiError::forbidden("namespaces is forbidden"));
        }
    }
    let clock = Arc::new(FakeClockPort::default());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry.clone()]));
    let manager = ClusterSessionManager::new(connector, source, clock.clone());
    manager.open(&entry, Default::default());
    futures::executor::block_on(manager.connect(&cluster)).expect("connect");
    let state = Arc::new(FakeStatePort::new());
    futures::executor::block_on(state.kv_set(
        &prefs_key(&cluster),
        serde_json::to_value(prefs).expect("prefs json"),
    ))
    .expect("store prefs");
    (NamespaceService::new(manager, state, clock), cluster)
}

fn render(service: NamespaceService, cluster: ClusterId, zoom: f32) -> anyhow::Result<RgbaImage> {
    let mut cx = headless();
    // The window grows with the zoom so the same content fits, as a user would size it.
    let window = cx.open_window(size(px(WIDTH * zoom), px(HEIGHT * zoom)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        oxikube_ui::set_ui_scale(cx, oxikube_ui::UiScale::new(zoom));
        oxikube_runtime::init_deterministic(cx);
        cx.set_reduce_motion(true);
        oxikube_catalog_ui::init(cx);
        cx.new(|cx| NamespaceSelector::new(cluster, service, window, cx))
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |view, window, cx| {
        let selector = view.downcast::<NamespaceSelector>().expect("the root view");
        selector.update(cx, |selector, cx| selector.open(window, cx));
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

fn check(name: &str, image: RgbaImage, zoom: f32) -> anyhow::Result<()> {
    let scale = HEADLESS_SCALE_FACTOR;
    anyhow::ensure!(
        image.dimensions()
            == (
                (WIDTH * zoom) as u32 * scale,
                (HEIGHT * zoom) as u32 * scale
            ),
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
    let open_prefs = || NamespacePrefs {
        selection: NamespaceSelection::from_names(["dev", "prod"]),
        favourites: ["prod", "kube-system"].into_iter().collect(),
        typed: Vec::new(),
    };
    let names = ["default", "dev", "kube-system", "prod", "stage"];
    let (open, cluster) = service(&names, false, &open_prefs());
    check("namespace_selector_open", render(open, cluster, 1.0)?, 1.0)?;
    // ui_scale 1.5 (E05-U557): row text, digits and badges grow with the rows.
    let (open, cluster) = service(&names, false, &open_prefs());
    check(
        "namespace_selector_open_scale1_5",
        render(open, cluster, 1.5)?,
        1.5,
    )?;

    let (restricted, cluster) = service(
        &[],
        true,
        &NamespacePrefs {
            selection: NamespaceSelection::single("team-a"),
            favourites: Default::default(),
            typed: vec!["team-a".into(), "team-b".into()],
        },
    );
    check(
        "namespace_selector_restricted",
        render(restricted, cluster, 1.0)?,
        1.0,
    )
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("namespace selector screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("namespace selector screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
