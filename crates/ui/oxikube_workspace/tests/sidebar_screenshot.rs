//! The cluster sidebar (E06-S10) for a cluster admin and for a restricted user, in light and dark.
//!
//! The admin sees all eleven sections (the cluster has a CRD, so Custom Resources shows); the
//! restricted user, who may only list pods and services, sees Cluster, Workloads and Network and a
//! muted "limited access" line. Every section but Workloads is collapsed so one frame shows the
//! whole list.
//!
//! Same harness and gate as `screenshot.rs` (`harness = false`, needs a GPU device, feature
//! `screenshot`, nightly job): `cargo test -p oxikube_workspace --features screenshot --test
//! sidebar_screenshot`. Goldens live in `tests/goldens/<os>/sidebar_*.png`; regenerate with
//! `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::{path::Path, process::ExitCode, sync::Arc};

use futures::executor::block_on;
use gpui::{HeadlessAppContext, px, size};
use oxikube_app::{ClusterSessionManager, IntegrationRegistry};
use oxikube_domain::access::{AccessRule, AccessRules};
use oxikube_domain::ids::{ContextName, Gvk};
use oxikube_domain::kinds::{ResourceKind, Verb};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
    headless::HEADLESS_SCALE_FACTOR,
    screenshot::{RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path},
};
use oxikube_ui::{Tokens, root::new_root};
use oxikube_workspace::sidebar::{SidebarDeps, SidebarPanel};

const WIDTH: f32 = 280.0;
const HEIGHT: f32 = 560.0;
const CLUSTER: &str = "3f2a9c1b7d4e8a60";
const COLLAPSED: [&str; 8] = [
    "nodes",
    "config",
    "network",
    "storage",
    "namespaces",
    "helm",
    "access-control",
    "custom-resources",
];

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn rules_for(restricted: bool) -> AccessRules {
    if restricted {
        [("", "pods"), ("", "services")].iter().fold(
            AccessRules::none(),
            |rules, (group, resource)| {
                rules.with_rule(AccessRule::granting(&["list"], &[group], &[resource], &[]))
            },
        )
    } else {
        AccessRules::all_access()
    }
}

fn render(tokens: Tokens, restricted: bool) -> anyhow::Result<RgbaImage> {
    let context = ClusterContext::new(
        CLUSTER.parse()?,
        ContextName::new("prod-eu"),
        SourceId("kubeconfig".into()),
    );
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let sessions = ClusterSessionManager::new(
        connector.clone(),
        Arc::new(FakeClusterSourcePort::new().with_contexts([context.clone()])),
        Arc::new(FakeClockPort::default()),
    );
    let ports = connector.ports_for(&context.cluster);
    ports.access.set_rules(rules_for(restricted));
    ports.discovery.set_kinds([ResourceKind {
        gvk: Gvk::new("argoproj.io", "v1alpha1", "Application"),
        preferred: true,
        plural: "applications".into(),
        singular: "application".into(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: [Verb::Get, Verb::List, Verb::Watch].into_iter().collect(),
        namespaced: true,
    }]);
    sessions.open(&context, Default::default());
    // The rules review of All namespaces cannot judge a restricted user, so a namespace is chosen.
    sessions.set_namespace_selection(
        &context.cluster,
        oxikube_domain::session::NamespaceSelection::single("dev"),
    )?;
    block_on(sessions.connect(&context.cluster))?;

    let deps = SidebarDeps {
        sessions,
        integrations: IntegrationRegistry::new(),
        state: Arc::new(FakeStatePort::new()),
        stores: None,
    };
    let mut cx = headless();
    let mut shown = None;
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, tokens);
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        oxikube_workspace::sidebar::init(cx);
        let panel = SidebarPanel::build(context.cluster.clone(), deps, cx);
        shown = Some(panel.clone());
        new_root(panel, window, cx)
    })?;
    cx.run_until_parked();
    // Collapse what the picture does not need, as the user would.
    let panel = shown.expect("the window was built");
    cx.update(|cx| {
        panel.update(cx, |panel, cx| {
            for id in COLLAPSED {
                panel.set_open(id, false, cx);
            }
        });
    });
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
    check("sidebar_admin_dark", render(Tokens::dark(), false)?)?;
    check("sidebar_admin_light", render(Tokens::light(), false)?)?;
    check("sidebar_restricted_dark", render(Tokens::dark(), true)?)?;
    check("sidebar_restricted_light", render(Tokens::light(), true)?)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("sidebar screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("sidebar screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
