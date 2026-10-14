//! The other sources (E14-S07) in the window: an update has no cluster and its "Open" button
//! dispatches `notifications::OpenLink`; a port-forward drop toasts and opens the forwarded pod;
//! an agent permission toast stays past the toast timeout and goes when the request is answered.

use gpui::{Entity, TestAppContext};
use oxikube_domain::command::Command;
use oxikube_domain::notification::NotificationKind;
use oxikube_testkit::{
    emit_agent_permission, emit_portforward_drop, emit_update_available, portforward_change,
};

use super::Fixture;
use crate::notifications::{APP_LABEL, TOAST_WINDOW};
use crate::toast::{ToastLayer, ToastLevel};

fn layer(fx: &mut Fixture) -> Entity<ToastLayer> {
    let ws = fx.ws.clone();
    fx.vcx.update(|_, cx| ws.read(cx).toast_layer().clone())
}

fn visible(fx: &mut Fixture) -> Vec<crate::toast::ToastSummary> {
    let layer = layer(fx);
    fx.vcx.update(|_, cx| layer.read(cx).visible())
}

#[gpui::test]
fn an_update_toasts_without_a_cluster_and_open_dispatches_the_link(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx);
    emit_update_available(&*fx.service.sources(), "0.6.1");
    fx.settle();

    let toasts = visible(&mut fx);
    assert_eq!(toasts.len(), 1);
    assert_eq!(toasts[0].level, ToastLevel::Info);
    assert_eq!(toasts[0].message.as_ref(), "Faster tables, fewer crashes");
    let rows = fx.wiring.centre.read_with(&fx.vcx, |c, _| c.rows());
    assert_eq!(rows[0].cluster, None);
    assert_eq!(rows[0].cluster_label.as_ref(), APP_LABEL);
    assert_eq!(rows[0].kind, NotificationKind::UpdateAvailable);

    fx.recorder.clear();
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    fx.click(&format!("toast-{}-action-0", toasts[0].id.as_u64()));
    let sent = fx.recorder.sent();
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(matches!(sent[0], Command::NotificationsMarkRead { .. }));
    assert_eq!(
        sent[1],
        Command::NotificationsOpenLink {
            url: "https://example.com/oxikube/releases/0.6.1".to_owned()
        }
    );
}

#[gpui::test]
fn a_port_forward_drop_toasts_and_opens_the_forwarded_pod(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx);
    let mut change = portforward_change(
        "fwd-1",
        oxikube_domain::notification::ForwardPhase::Active,
        oxikube_domain::notification::ForwardPhase::Reconnecting,
    );
    change.cluster = super::prod();
    change.target = Some(super::pod("api-0"));
    fx.service.sources().port_forward_changed(change);
    fx.settle();

    let toasts = visible(&mut fx);
    assert_eq!(toasts.len(), 1);
    assert_eq!(toasts[0].level, ToastLevel::Warning);
    assert_eq!(toasts[0].message.as_ref(), "connection reset");
    fx.recorder.clear();
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    fx.click(&format!("toast-{}-action-0", toasts[0].id.as_u64()));
    assert!(fx.recorder.sent().contains(&Command::ResourceOpen {
        target: super::pod("api-0")
    }));
}

#[gpui::test]
fn a_permission_toast_stays_until_the_request_is_answered(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx);
    let hooks = fx.service.sources();
    // The fixture's cluster is the testkit's hook cluster only by name; no cluster is needed.
    hooks.agent_permission_requested(oxikube_domain::notification::AgentPermissionRequest {
        cluster: None,
        ..oxikube_testkit::agent_permission("s1", "r1")
    });
    fx.settle();
    assert_eq!(visible(&mut fx).len(), 1);
    assert_eq!(fx.badge(), 1);

    // Past every timeout the toast is still there.
    fx.vcx.executor().advance_clock(TOAST_WINDOW * 10);
    fx.vcx.run_until_parked();
    assert_eq!(visible(&mut fx).len(), 1, "it does not time out");

    hooks.agent_permission_resolved("s1", "r1");
    fx.settle();
    assert!(visible(&mut fx).is_empty(), "answered: the toast goes");
    assert_eq!(fx.badge(), 0, "and the row is read");
    assert!(fx.service.list()[0].read);
}

#[gpui::test]
fn a_dropped_port_forward_toast_times_out_like_any_other(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx);
    emit_portforward_drop(&*fx.service.sources(), "fwd-1");
    emit_agent_permission(&*fx.service.sources(), "s1", "r1");
    fx.settle();
    assert_eq!(visible(&mut fx).len(), 2);
    fx.vcx.executor().advance_clock(TOAST_WINDOW * 10);
    fx.vcx.run_until_parked();
    let left = visible(&mut fx);
    assert_eq!(left.len(), 1, "only the permission request stays: {left:?}");
    assert_eq!(
        left[0].message.as_ref(),
        "Scale deployment web to 0 replicas"
    );
}
