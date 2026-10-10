//! The container picker (E09-S08): a pod with three containers, the annotated default selected,
//! in the dark and the light theme; and the debug dialog (E09-S10) with its defaults, in both.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, WeakEntity, px, size};
use oxikube_app::command_bus::{CommandBus, CommandRegistry};
use oxikube_app::exec::ExecContainer;
use oxikube_app::{
    ClusterSessionManager, ContainerChoices, DebugDefaults, DebugRunner, ExecService, MutationGuard,
};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::view::ContainerKind;
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_resources_ui::exec::{ContainerPickerDelegate, DebugDialog, ExecFlow, ExecKind};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
    screenshot::RgbaImage,
};

use super::{Ignore, check, headless};

const WIDTH: f32 = 420.0;
const HEIGHT: f32 = 172.0;

fn container(name: &str, kind: ContainerKind, running: bool) -> ExecContainer {
    ExecContainer {
        name: name.into(),
        kind,
        running,
    }
}

fn render(light: bool) -> anyhow::Result<RgbaImage> {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new().with_contexts([entry])),
        Arc::new(FakeClockPort::default()),
    );
    let target = ResourceRef::namespaced(cluster, Gvk::new("", "v1", "Pod"), "shop", "web-6a4f");
    let choices = ContainerChoices {
        containers: vec![
            container("app", ContainerKind::Regular, true),
            container("envoy", ContainerKind::Regular, true),
            container("log-shipper", ContainerKind::Sidecar, false),
        ],
        preselected: 1,
    };
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        let tokens = if light {
            oxikube_ui::Tokens::light()
        } else {
            oxikube_ui::Tokens::dark()
        };
        oxikube_ui::set_tokens(cx, tokens);
        oxikube_runtime::init_deterministic(cx);
        cx.set_reduce_motion(true);
        let flow = ExecFlow::new(
            Arc::new(ExecService::new(sessions)),
            Rc::new(Ignore),
            WeakEntity::new_invalid(),
        );
        cx.new(|cx| {
            ContainerPickerDelegate::picker(ExecKind::Shell, target, choices, flow, window, cx)
        })
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

const DEBUG_WIDTH: f32 = 560.0;
const DEBUG_HEIGHT: f32 = 510.0;

/// The debug dialog of a pod with two containers, with the defaults a user sees first.
fn render_debug(light: bool) -> anyhow::Result<RgbaImage> {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new().with_contexts([entry])),
        clock.clone(),
    );
    let bus = CommandBus::new(
        CommandRegistry::new(),
        MutationGuard::new(sessions, Arc::new(FakeStatePort::new()), clock),
    );
    let pod = ResourceRef::namespaced(cluster, Gvk::new("", "v1", "Pod"), "shop", "web-6a4f");
    let defaults = DebugDefaults {
        pod,
        image: "busybox".into(),
        command: "sh".into(),
        targets: vec![
            container("app", ContainerKind::Regular, true),
            container("envoy", ContainerKind::Regular, true),
        ],
        target: 0,
    };
    let mut cx = headless();
    let window = cx.open_window(size(px(DEBUG_WIDTH), px(DEBUG_HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        let tokens = if light {
            oxikube_ui::Tokens::light()
        } else {
            oxikube_ui::Tokens::dark()
        };
        oxikube_ui::set_tokens(cx, tokens);
        oxikube_runtime::init_deterministic(cx);
        cx.set_reduce_motion(true);
        let runner = DebugRunner::new(bus, "me");
        cx.new(|cx| DebugDialog::new(defaults, runner, WeakEntity::new_invalid(), window, cx))
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

pub(crate) fn run() -> anyhow::Result<()> {
    check("exec_picker_dark", render(false)?, WIDTH, HEIGHT)?;
    check("exec_picker_light", render(true)?, WIDTH, HEIGHT)?;
    check(
        "exec_debug_dark",
        render_debug(false)?,
        DEBUG_WIDTH,
        DEBUG_HEIGHT,
    )?;
    check(
        "exec_debug_light",
        render_debug(true)?,
        DEBUG_WIDTH,
        DEBUG_HEIGHT,
    )
}
