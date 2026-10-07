//! The header of a pod's detail: Shell and Attach, through the same flow as the table.

use gpui::TestAppContext;
use oxikube_domain::command::Command;

use super::{pod_with, sent_exec};
use crate::detail::tests::fixture::{Detail, pod_ref, web_pod, web_replicaset};

#[gpui::test]
fn a_pods_header_has_shell_and_attach_that_dispatch_with_the_container(cx: &mut TestAppContext) {
    let mut d = Detail::with_exec(cx, [web_pod(), web_replicaset()]);
    d.open(&pod_ref("web-0"));
    assert!(d.shown("detail-shell") && d.shown("detail-attach"));
    d.f.dispatcher.clear();
    d.click("detail-shell");
    d.click("detail-attach");
    let sent = sent_exec(&d.f);
    // The test pod has one container, `app`: no question.
    assert!(
        matches!(
            sent.as_slice(),
            [Command::PodShell { container: Some(a), .. }, Command::PodAttach { container: Some(b), .. }]
                if a == "app" && b == "app"
        ),
        "{sent:?}"
    );
}

#[gpui::test]
fn the_header_asks_which_container_when_the_pod_has_several(cx: &mut TestAppContext) {
    let mut d = Detail::with_exec(cx, [pod_with("shop", "web-0", &["app", "proxy"])]);
    d.open(&pod_ref("web-0"));
    d.f.dispatcher.clear();
    d.click("detail-shell");
    assert!(sent_exec(&d.f).is_empty(), "asked, not sent");
    let tabs = d.f.tabs.clone();
    let open = d.f.vcx.update(|_, cx| {
        let tab = tabs
            .read(cx)
            .tab(&crate::table::tests::fixture::cluster())
            .cloned()?;
        let layer = tab.read(cx).workspace().read(cx).modal_layer().clone();
        layer
            .read(cx)
            .active_modal::<crate::exec::ContainerPicker>()
    });
    assert!(open.is_some(), "the picker is open in the tab's workspace");
}

#[gpui::test]
fn other_kinds_have_no_such_buttons_and_a_read_only_cluster_disables_them(cx: &mut TestAppContext) {
    let mut d = Detail::with_exec(cx, [web_pod(), web_replicaset()]);
    d.open(&crate::detail::tests::fixture::pod_ref("web-0"));
    d.f.sessions
        .set_read_only(&crate::table::tests::fixture::cluster(), true)
        .unwrap();
    d.settle();
    d.f.dispatcher.clear();
    d.click("detail-shell");
    assert!(
        sent_exec(&d.f).is_empty(),
        "a disabled button sends nothing"
    );

    let rs = ResourceRefExt::replicaset();
    d.open(&rs);
    assert!(!d.shown("detail-shell"), "a ReplicaSet has no shell");
}

struct ResourceRefExt;

impl ResourceRefExt {
    fn replicaset() -> oxikube_domain::ids::ResourceRef {
        oxikube_domain::ids::ResourceRef::namespaced(
            crate::table::tests::fixture::cluster(),
            oxikube_domain::ids::Gvk::new("apps", "v1", "ReplicaSet"),
            "shop",
            "web-5d",
        )
    }
}
