//! The Service panel's Forward buttons (E15-S04): one per TCP port, opening the start dialog on
//! that port; absent without a port-forward manager; disabled with the reason on a read-only
//! cluster.

use gpui::TestAppContext;
use oxikube_portforward_ui::StartDialog;

use super::{service_ref, shop};
use crate::detail::tests::fixture::Detail;

fn window(cx: &mut TestAppContext) -> Detail {
    let mut d = Detail::with_forward(cx, shop());
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    d
}

#[gpui::test]
fn a_forward_button_per_tcp_port_opens_the_dialog_on_that_port(cx: &mut TestAppContext) {
    let mut d = window(cx);
    d.open(&service_ref("web"));
    assert!(
        d.shown("service-forward-80"),
        "TCP port 80 can be forwarded"
    );
    assert!(!d.shown("service-forward-53"), "UDP port 53 cannot");

    d.click("service-forward-80");
    let dialog = d
        .modal::<StartDialog>()
        .expect("Forward opens the start dialog");
    let input = d.f.vcx.update(|_, cx| dialog.read(cx).input(cx));
    assert_eq!(input.remote_port, "80", "prefilled with the clicked port");
    assert!(d.shown("pf-port-0"), "the Service's ports are offered");
    assert!(
        d.f.forward.as_ref().unwrap().calls.lock().is_empty(),
        "opening sends nothing"
    );
}

#[gpui::test]
fn no_port_forward_manager_means_no_forward_button(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, shop());
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    d.open(&service_ref("web"));
    assert!(d.shown("service-port-0"), "the ports are still listed");
    assert!(!d.shown("service-forward-80"));
}

#[gpui::test]
fn a_read_only_cluster_disables_the_button_and_says_why(cx: &mut TestAppContext) {
    let mut d = window(cx);
    let view = d.open(&service_ref("web"));
    let rig = d.f.forward.clone().unwrap();
    rig.read_only
        .store(true, std::sync::atomic::Ordering::SeqCst);
    d.update(&view, |_, cx| cx.notify());
    let state = d.read(&view, |v| v.forward_state());
    let reason = state.flatten().expect("blocked");
    assert!(reason.contains("allowed_in_readonly"), "{reason}");
    assert!(d.shown("service-forward-80"), "the button is still drawn");
    d.click("service-forward-80");
    assert!(
        d.modal::<StartDialog>().is_none(),
        "a disabled button opens nothing"
    );
}
