//! The Node and Namespace allocation views (E13-S11) in the app against a real cluster: the real
//! init order, its main window headless, kind connected; `resource::Open` for the cluster's node
//! and for a namespace of the test's own, through the command bus as the palette and the tables
//! send it.
//!
//! * The node's drawer shows the allocation section of the pods on the real node, and the usage
//!   segment: a bar when metrics-server answers (the kind cluster installs it), otherwise the
//!   visible reason.
//! * A namespace with a `ResourceQuota` and a running pod shows its totals against the quota, the
//!   quota's own lines, and the usage segment.
//!
//! The namespace and its objects are the test's own (`oxi-test-<rand>`, deleted at the end); the
//! node is only read. `cargo test -p oxikube --features integration --test kind_allocation` with
//! `OXIKUBE_TEST_CONTEXT` set (`cargo xtask kind-up`); without it the test returns at once. Real
//! I/O wakes GPUI tasks from Tokio threads, so it allows parking and polls with short real-time
//! sleeps.
#![cfg(feature = "integration")]

mod kind_common;

use std::process::{Command, Stdio};

use gpui::{TestAppContext, VisualTestContext};
use oxikube::app_state::AppState;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command as BusCommand;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_testkit::integration::pods;
use oxikube_testkit::integration::{TestNamespace, ensure_kind_context, test_context};

use kind_common::{create_pod_from, launch, on_cluster, wait};

/// Opens `target`'s detail the way the palette and the tables do: `resource::Open` on the bus.
fn open(vcx: &mut VisualTestContext, target: ResourceRef) {
    let bus = vcx
        .update(|_, cx| AppState::global(cx).command_bus().cloned())
        .expect("the mount set the bus");
    let outcome = futures::executor::block_on(bus.dispatch(
        BusCommand::ResourceOpen { target },
        DispatchContext::new(Initiator::Ui, "kind test"),
    ));
    assert!(outcome.is_ok(), "{outcome:?}");
}

/// Lets the panels' timers fire: the test clock only moves when it is advanced (the panels look at
/// the store on a quarter-second tick and poll metrics on their own timer).
fn tick(vcx: &mut VisualTestContext) {
    vcx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
}

/// Draws a frame and whether the element tagged `selector` is on screen.
fn drawn(vcx: &mut VisualTestContext, selector: &'static str) -> bool {
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    vcx.debug_bounds(selector).is_some()
}

fn node_name(context: &str) -> String {
    on_cluster(context, async |client| {
        let resource = kube::api::ApiResource::from_gvk_with_plural(
            &kube::api::GroupVersionKind::gvk("", "v1", "Node"),
            "nodes",
        );
        kube::Api::<kube::api::DynamicObject>::all_with(client, &resource)
            .list(&Default::default())
            .await
            .expect("list nodes")
            .items
            .into_iter()
            .filter_map(|n| n.metadata.name)
            // Other suites create fake nodes no kubelet backs.
            .find(|n| !n.starts_with("oxi-fake-"))
            .expect("a real node")
    })
}

fn kubectl_apply(context: &str, manifest: &serde_json::Value) {
    use std::io::Write as _;
    let mut child = Command::new("kubectl")
        .args(["--context", context, "apply", "-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run kubectl apply");
    child
        .stdin
        .take()
        .expect("kubectl's stdin")
        .write_all(manifest.to_string().as_bytes())
        .expect("write the manifest");
    let out = child.wait_with_output().expect("kubectl apply");
    assert!(
        out.status.success(),
        "kubectl apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Whether metrics-server answers (`kubectl top nodes` works).
fn metrics_server_answers(context: &str) -> bool {
    Command::new("kubectl")
        .args(["--context", context, "top", "nodes"])
        .output()
        .is_ok_and(|out| out.status.success())
}

#[gpui::test]
fn the_node_and_namespace_panels_show_allocation_in_the_app(cx: &mut TestAppContext) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    cx.executor().allow_parking();
    let ns = TestNamespace::create(&context).expect("namespace");
    kubectl_apply(
        &context,
        &serde_json::json!({
            "apiVersion": "v1", "kind": "ResourceQuota",
            "metadata": {"name": "compute", "namespace": ns.name()},
            "spec": {"hard": {"requests.cpu": "2", "limits.cpu": "4", "pods": "10"}},
        }),
    );
    let mut pod = pods::sleeper("alloc-a");
    pod["spec"]["containers"][0]["resources"] = serde_json::json!({
        "requests": {"cpu": "100m", "memory": "32Mi"},
        "limits": {"cpu": "200m", "memory": "64Mi"},
    });
    create_pod_from(&context, ns.name(), pod);
    let has_metrics = metrics_server_answers(&context);

    let mut launched = launch(cx, &context);
    let cluster: ClusterId = launched.cluster.clone();
    let vcx = &mut launched.vcx;

    // The node: requests and limits of its pods, and usage (or why there is none).
    let node = ResourceRef::cluster_scoped(
        cluster.clone(),
        Gvk::new("", "v1", "Node"),
        node_name(&context),
    );
    open(vcx, node);
    wait(vcx, "the node's allocation section to be drawn", |vcx| {
        drawn(vcx, "node-allocation-cpu-requests") && drawn(vcx, "node-allocation-memory-requests")
    });
    assert!(drawn(vcx, "node-allocation-pods-requests"));
    assert!(
        drawn(vcx, "node-allocation-cpu-limits-text"),
        "the real node's pods carry CPU limits or requests, and a limits line is drawn"
    );
    if has_metrics {
        wait(
            vcx,
            "metrics-server's reading to reach the node's bars",
            |vcx| drawn(vcx, "node-allocation-cpu-usage"),
        );
    } else {
        wait(vcx, "the node to say why there is no usage", |vcx| {
            drawn(vcx, "node-allocation-cpu-usage-text")
        });
    }

    // The namespace: its pod against the quota, the quota's own lines, the usage segment.
    let target = ResourceRef::cluster_scoped(cluster, Gvk::new("", "v1", "Namespace"), ns.name());
    open(vcx, target);
    wait(vcx, "the namespace's allocation against its quota", |vcx| {
        tick(vcx);
        drawn(vcx, "namespace-allocation-cpu-bar") && drawn(vcx, "namespace-quota-0-0")
    });
    assert!(drawn(vcx, "namespace-allocation-cpu-requests"));
    assert!(drawn(vcx, "namespace-allocation-pods-requests"));
    assert!(
        !drawn(vcx, "namespace-allocation-versus"),
        "there is a quota: no totals-only note"
    );
    wait(vcx, "the namespace's usage line", |vcx| {
        tick(vcx);
        drawn(vcx, "namespace-allocation-cpu-usage-text")
    });
}
