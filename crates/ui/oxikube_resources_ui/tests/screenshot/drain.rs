//! The drain dialog (E12-S10), in the dark and the light theme: the plan of a node with a
//! ReplicaSet pod, a DaemonSet pod and an unmanaged one that blocks the drain
//! (`drain_plan_*`), and the drain under way with `--force`: one pod gone, one held back by a
//! PodDisruptionBudget (`drain_progress_*`). The drain runs through the real bus and guard over
//! a scripted drain port.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{AppContext as _, WeakEntity, px, size};
use oxikube_app::command_bus::CommandRegistry;
use oxikube_app::nodes::{DrainFlow, NodeService};
use oxikube_app::{ClusterSessionManager, CommandBus, MutationGuard};
use oxikube_domain::drain::{DrainProgress, PodRef, SkipReason, SkippedPod};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_ports::{ClusterContext, ListPage, SourceId};
use oxikube_resources_ui::kinds::node::drain::{DrainDialog, DrainSwitch};
use oxikube_testkit::{
    DrainRun, FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeDrainPort,
    FakeStatePort, PodBuilder, node, pod, screenshot::RgbaImage,
};

use super::{Ignore, check, headless};

const WIDTH: f32 = 640.0;
const HEIGHT: f32 = 500.0;

fn pin(cx: &mut gpui::App, light: bool) {
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
}

fn pod_ref(namespace: &str, name: &str) -> PodRef {
    PodRef {
        namespace: namespace.into(),
        name: name.into(),
        uid: None,
    }
}

/// `web-7d9f` gone, `lone` blocked by a budget, the DaemonSet's `agent` skipped; then it waits.
fn under_way() -> DrainRun {
    let (web, lone) = (pod_ref("shop", "web-7d9f"), pod_ref("shop", "lone"));
    DrainRun::of([
        DrainProgress::Planned {
            evict: vec![web.clone(), lone.clone()],
            skipped: vec![SkippedPod {
                pod: pod_ref("kube-system", "agent-x2k4"),
                reason: SkipReason::DaemonSet,
            }],
        },
        DrainProgress::Cordoned { already: false },
        DrainProgress::Evicting {
            pod: web.clone(),
            attempt: 1,
        },
        DrainProgress::Evicted { pod: web.clone() },
        DrainProgress::Gone { pod: web },
        DrainProgress::Evicting {
            pod: lone.clone(),
            attempt: 2,
        },
        DrainProgress::Blocked {
            pod: lone,
            attempt: 2,
            reason: "The disruption budget lone-pdb needs 1 healthy pod and has 1.".into(),
            retry_in: Duration::from_secs(4),
        },
    ])
    .hold()
}

fn render(light: bool, run: bool) -> anyhow::Result<RgbaImage> {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let ports = connector.ports_for(&cluster);
    ports.resources.insert(node().name("worker-2").build());
    let on = |b: PodBuilder| b.node("worker-2").running().build();
    ports
        .resources
        .script()
        .list
        .push_ok(ListPage::complete(vec![
            on(pod()
                .namespace("kube-system")
                .name("agent-x2k4")
                .controlled_by("DaemonSet", "agent")),
            on(pod().namespace("shop").name("lone")),
            on(pod()
                .namespace("shop")
                .name("web-7d9f")
                .controlled_by("ReplicaSet", "web")),
        ]));
    ports.resources.script().evict.push_ok(());
    let clock = Arc::new(FakeClockPort::default());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster))?;

    let port = Arc::new(FakeDrainPort::new());
    port.push(under_way());
    let nodes = Arc::new(NodeService::new(port, sessions.clone(), clock.clone()));
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::nodes", |r| {
            oxikube_app::nodes::register_drain_command(r, nodes.clone())
        })
        .expect("register node::Drain");
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(sessions, Arc::new(FakeStatePort::new()), clock),
    );
    nodes.set_audit(bus.guard().audit_handle());
    let flow = DrainFlow::new(bus, nodes, "alice");
    let target = ResourceRef::cluster_scoped(cluster, Gvk::new("", "v1", "Node"), "worker-2");

    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        pin(cx, light);
        cx.new(|cx| {
            DrainDialog::new(
                flow,
                target,
                WeakEntity::new_invalid(),
                Rc::new(Ignore),
                window,
                cx,
            )
        })
    })?;
    cx.run_until_parked();
    if run {
        cx.update_window(window.into(), |view, window, cx| {
            let dialog = view.downcast::<DrainDialog>().expect("the root view");
            dialog.update(cx, |dialog, cx| {
                dialog.set_flag(DrainSwitch::Force, true, cx);
                dialog.type_name("worker-2", window, cx);
            });
        })?;
        cx.run_until_parked();
        cx.update_window(window.into(), |view, _, cx| {
            let dialog = view.downcast::<DrainDialog>().expect("the root view");
            dialog.update(cx, |dialog, cx| dialog.start(cx));
        })?;
        cx.run_until_parked();
        // The steps land through a coalesced notify: one frame-pacer interval.
        cx.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
    }
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

pub(crate) fn run() -> anyhow::Result<()> {
    check("drain_plan_dark", render(false, false)?, WIDTH, HEIGHT)?;
    check("drain_plan_light", render(true, false)?, WIDTH, HEIGHT)?;
    check("drain_progress_dark", render(false, true)?, WIDTH, HEIGHT)?;
    check("drain_progress_light", render(true, true)?, WIDTH, HEIGHT)
}
