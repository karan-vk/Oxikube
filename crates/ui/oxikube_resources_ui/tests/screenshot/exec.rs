//! The container picker (E09-S08): a pod with three containers, the annotated default selected,
//! in the dark and the light theme.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, WeakEntity, px, size};
use oxikube_app::exec::ExecContainer;
use oxikube_app::{ClusterSessionManager, ContainerChoices, ExecService};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::view::ContainerKind;
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_resources_ui::exec::{ContainerPicker, ExecFlow, ExecKind};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, screenshot::RgbaImage,
};

use super::{Ignore, check, headless};

const WIDTH: f32 = 420.0;
const HEIGHT: f32 = 280.0;

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
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |_, cx| {
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
        cx.new(|cx| ContainerPicker::new(ExecKind::Shell, target, choices, flow, cx))
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

pub(crate) fn run() -> anyhow::Result<()> {
    check("exec_picker_dark", render(false)?, WIDTH, HEIGHT)?;
    check("exec_picker_light", render(true)?, WIDTH, HEIGHT)
}
