//! Screenshots of the resource views, rendered through `Window::render_to_image`.
//!
//! - `pods_table_tones`, `pods_table_tones_light`: running, pending, container-creating,
//!   crash-looping, failed, succeeded and terminating pods, one row selected; the Status cells in
//!   the theme's `oxikube` colours, in the dark and the light theme (E07-S12, the epic's visual
//!   baseline).
//! - `detail_deployment_dark`, `detail_deployment_light` (`detail`): the detail drawer of a
//!   Deployment in both themes: header with status chip, owner-less metadata, conditions, status.
//! - `detail_yaml_dark`, `detail_yaml_light`, `detail_describe_dark` (`detail`): the YAML tab (read-only,
//!   tree-sitter highlighted) and the Describe tab of the same Deployment (E07-S06).
//!
//! - `pods_table_filtered`: the same pods with `cart|checkout|web` typed in the filter bar: three
//!   rows and the `3 of 7` count (E07-S04).
//! - `pods_table_one_namespace`: the same pods with the namespace `shop` selected in a window too
//!   narrow for every default column: no Namespace column, Status before Ready, and the fade at
//!   the right edge that says Node and IP continue past the view (E07-U561).
//! - `detail_secret_tls_dark`, `detail_secret_registry_dark`, `detail_secret_revealed_light`, `detail_secret_yaml_decoded_dark`, `detail_configmap_preview_dark` (`secrets`): the Secret detail masked with its type's facts, revealed, its YAML tab decoded, and a ConfigMap's keys with an expanded preview (E12-S11)
//! - `detail_crd_schema_dark`, `detail_crd_schema_light` (`crd`): the Schema tab of a CRD's detail
//!   (E07-S07) with `spec` and `spec.containers` open.
//!
//! - `detail_quota_dark`, `detail_quota_light`, `detail_webhook_dark` (`config`): the Config kinds' panels
//!   above the Overview (E12-S12): a ResourceQuota's used-against-hard bars and a webhook
//!   configuration with its `caBundle` shown as present or absent.
//!
//! - `detail_service_dark`, `detail_service_light` (`service`): the detail drawer of a Service with
//!   its ports ("Open in browser" on the TCP ones), EndpointSlices with ready counts and backing
//!   pods (E12-S13).
//! - `network_ingress_*`, `network_policy_*`, `network_httproute_*` (`network`, dark and light): the Network
//!   panels of an Ingress (rules table, TLS secret, load balancer), a NetworkPolicy (peers, ports, the
//!   IP block exception, default-deny egress) and an HTTPRoute (the parent that refused it, the
//!   unresolved backend), with the generic rows below (E12-S14).
//! - `detail_node_dark`, `detail_node_light` (`node`): the detail drawer of a cordoned worker
//!   (E12-S09): the badge, conditions, the allocation bars from the pods on the node, taints,
//!   pods, images, capacity and system info.
//! - `drain_plan_dark`, `drain_plan_light`, `drain_progress_dark`, `drain_progress_light` (`drain`):
//!   the drain dialog of a node with a pod that blocks the drain (the plan, nothing changed), and
//!   the drain under way with `--force`: one pod deleted, one blocked by a PodDisruptionBudget
//!   (E12-S10).
//! - `exec_picker_dark`, `exec_picker_light` (`exec`): the container picker of a pod with three
//!   containers, the default one selected (E09-S08; the generic picker of E11-S02 since).
//! - `exec_debug_dark`, `exec_debug_light` (`exec`): the debug-container dialog of a pod with two
//!   containers, with the defaults (busybox, `sh`) and the note that the container is permanent
//!   (E09-S10).
//! - `detail_metrics_pod_dark`, `detail_metrics_pod_light`, `detail_metrics_error_dark` (`metrics`):
//!   the Metrics tab of a pod with two containers (a line each, requests and limits as reference
//!   lines, the range selector) over a scripted Prometheus, and the same tab when Prometheus is
//!   unreachable (the reason inline with Retry, no chart frame) (E13-S08).
//! - `pods_columns_dark`, `detail_pod_containers_dark`, `detail_pod_containers_light` (`pods`):
//!   the Pod kind's own columns with live metrics toned by the thresholds (E13-S03), and a pod's
//!   detail opening on its containers sub-table with each container's usage (E12-S02, E13-S03);
//!   `pods_metrics_unavailable_dark`: the `metrics unavailable: <reason>` badge.
//!
//! - `namespaces_table_dark`, `namespaces_table_light` (`namespaces`): the Namespaces table with its
//!   Pods column (E12-S16); `namespaces_delete_dark`, `namespaces_delete_light`: the delete dialog
//!   of a namespace with HNC children and a lot in it; `namespaces_create_dark`: the create dialog
//!   showing an invalid name.
//! - `detail_namespace_dark`, `detail_namespace_light`, `detail_namespace_unavailable_dark`
//!   (`namespace_allocation`): the namespace panel's allocation section (E13-S11): requests, limits
//!   and live usage against a ResourceQuota, the quota's own used-against-hard lines, and the
//!   same namespace without metrics-server and without a quota. `detail_node_dark`,
//!   `detail_node_light` and `detail_node_unavailable_dark` (`node`) show the node's.
//! - `storage_claims_table_*`, `storage_claim_panel_*`, `storage_resize_dialog_*` (`storage`): the
//!   claims table with its Pods (used by) column, a claim's detail panel with the Pods that mount
//!   it, and the resize dialog refusing a shrink, in both themes (E12-S15).
//! - `bulk_label_editor_*`, `bulk_delete_confirm_*`, `bulk_results_*` (`bulk`): the bulk dialog over a selection (E12-S19): the label editor with its preview and the selector warning, the delete
//!   confirmation with counts by kind and namespace and the cluster name to type, and the per-row results of a batch with a failed and a skipped row.
//!
//! - `detail_used_by_dark`, `detail_used_by_light` (`used_by`): the Used by tab of a ConfigMap's detail:
//!   the summary and the workloads that use it (a Deployment through its ReplicaSet, a StatefulSet, a
//!   CronJob and a lone Pod) (E12-S18).
//!
//! - `pods_table_warning_badges_dark`, `pods_table_warning_badges_light`, `detail_events_tab_dark`,
//!   `detail_events_tab_light` (`warnings`): the warning icon column of the pods table (rows with a
//!   `Warning` event within the hour) and a pod's Events tab over the app's events index (E14-S04).
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
use oxikube_domain::session::NamespaceSelection;
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

#[path = "screenshot/bulk.rs"]
mod bulk;
#[path = "screenshot/config.rs"]
mod config;
#[path = "screenshot/crd.rs"]
mod crd;
#[path = "screenshot/detail.rs"]
mod detail;
#[path = "screenshot/drain.rs"]
mod drain;
#[path = "screenshot/exec.rs"]
mod exec;
#[path = "screenshot/metrics.rs"]
mod metrics;
#[path = "screenshot/namespace_allocation.rs"]
mod namespace_allocation;
#[path = "screenshot/namespaces.rs"]
mod namespaces;
#[path = "screenshot/network.rs"]
mod network;
#[path = "screenshot/node.rs"]
mod node;
#[path = "screenshot/pods.rs"]
mod pods;
#[path = "screenshot/secrets.rs"]
mod secrets;
#[path = "screenshot/service.rs"]
mod service;
#[path = "screenshot/storage.rs"]
mod storage;
#[path = "screenshot/used_by.rs"]
mod used_by;
#[path = "screenshot/warnings.rs"]
mod warnings;

const WIDTH: f32 = 960.0;
/// Narrower than the default columns (Name, Status, Ready, Node, IP: 740 px).
const NARROW: f32 = 600.0;
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

/// Renders the table in the dark or `light` theme, with `filter` typed in the bar when given.
fn render(filter: Option<&str>, light: bool) -> anyhow::Result<RgbaImage> {
    render_at(filter, light, WIDTH, None)
}

/// [`render`] in a window `width` wide, with `namespace` selected when given.
fn render_at(
    filter: Option<&str>,
    light: bool,
    width: f32,
    namespace: Option<&str>,
) -> anyhow::Result<RgbaImage> {
    let (sessions, cluster, state, clock) = fixture();
    if let Some(namespace) = namespace {
        sessions
            .set_namespace_selection(&cluster, NamespaceSelection::single(namespace))
            .expect("open session");
    }
    let mut cx = headless();
    let window = cx.open_window(size(px(width), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        let tokens = if light {
            oxikube_ui::Tokens::light()
        } else {
            oxikube_ui::Tokens::dark()
        };
        oxikube_ui::set_tokens(cx, tokens);
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
    cx.update_window(window.into(), |view, window, cx| {
        let table = view.downcast::<ResourceTable>().expect("the root view");
        table.update(cx, |table, cx| {
            if let Some(filter) = filter {
                table.set_filter_text(filter, window, cx);
            }
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
        render(None, false).and_then(|image| check("pods_table_tones", image, WIDTH, HEIGHT)),
        render(None, true).and_then(|image| check("pods_table_tones_light", image, WIDTH, HEIGHT)),
        render(Some("cart|checkout|web"), false)
            .and_then(|image| check("pods_table_filtered", image, WIDTH, HEIGHT)),
        render_at(None, false, NARROW, Some("shop"))
            .and_then(|image| check("pods_table_one_namespace", image, NARROW, HEIGHT)),
        detail::run(),
        config::run(),
        secrets::run(),
        crd::run(),
        network::run(),
        exec::run(),
        node::run(),
        drain::run(),
        pods::run(),
        metrics::run(),
        service::run(),
        namespaces::run(),
        namespace_allocation::run(),
        storage::run(),
        used_by::run(),
        bulk::run(),
        warnings::run(),
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
