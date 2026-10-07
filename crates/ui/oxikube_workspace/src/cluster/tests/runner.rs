//! The command runner end to end: toasts naming the cluster, the pulse, the confirmation
//! dialog, and lifting read-only through the toggle.

use std::time::Duration;

use gpui::TestAppContext;
use oxikube_domain::ClusterPreset;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ContextName, Gvk, ResourceRef};

use super::{Fixture, bounds, fixture};
use crate::cluster::tests::fixture::{PROD, id};
use crate::toast::ToastLevel;

fn delete_pod() -> Command {
    Command::PodDelete {
        target: ResourceRef::namespaced(id(PROD), Gvk::new("", "v1", "Pod"), "default", "web-0"),
        grace_period_seconds: None,
    }
}

fn run(f: &mut Fixture, command: Command) {
    let runner = f.runner.clone();
    f.vcx.update(|window, cx| runner.run(command, window, cx));
    f.vcx.run_until_parked();
}

fn toasts(f: &mut Fixture) -> Vec<(ToastLevel, String)> {
    let layer = f.vcx.update(|_, cx| f.ws.read(cx).toast_layer().clone());
    f.vcx.update(|_, cx| {
        layer
            .read(cx)
            .visible()
            .iter()
            .map(|t| (t.level, t.message.to_string()))
            .collect()
    })
}

fn modal_open(f: &mut Fixture) -> bool {
    let layer = f.vcx.update(|_, cx| f.ws.read(cx).modal_layer().clone());
    f.vcx.update(|_, cx| layer.read(cx).has_active_modal())
}

#[gpui::test]
fn a_denied_mutation_shows_a_toast_naming_the_cluster_and_pulses_the_badge_once(
    cx: &mut TestAppContext,
) {
    let mut f = fixture(cx);
    f.vcx.update(|_, cx| cx.set_reduce_motion(false));
    f.manager.set_read_only(&id(PROD), true).unwrap();
    f.vcx.run_until_parked();

    run(&mut f, delete_pod());
    run(&mut f, delete_pod());
    run(&mut f, delete_pod());

    let shown = toasts(&mut f);
    assert_eq!(
        shown.len(),
        1,
        "repeated refusals share one toast: {shown:?}"
    );
    assert_eq!(shown[0].0, ToastLevel::Warning);
    assert!(shown[0].1.contains("prod-eu"), "{}", shown[0].1);
    assert!(shown[0].1.contains("read-only"), "{}", shown[0].1);
    assert_eq!(*f.deletes.lock(), 0, "the handler never ran");
    assert_eq!(
        f.vcx.update(|_, cx| f.status.read(cx).pulse_count()),
        1,
        "three refusals in a burst pulse once"
    );

    // Once the pulse is over, the next refusal may pulse again.
    f.vcx.executor().advance_clock(Duration::from_millis(400));
    f.vcx.run_until_parked();
    run(&mut f, delete_pod());
    assert_eq!(f.vcx.update(|_, cx| f.status.read(cx).pulse_count()), 2);
}

#[gpui::test]
fn reduce_motion_turns_the_pulse_off_but_not_the_toast(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    f.manager.set_read_only(&id(PROD), true).unwrap();
    f.vcx.run_until_parked();
    run(&mut f, delete_pod());
    assert_eq!(f.vcx.update(|_, cx| f.status.read(cx).pulse_count()), 0);
    assert_eq!(toasts(&mut f).len(), 1);
}

#[gpui::test]
fn a_writable_cluster_asks_to_confirm_a_mutation_instead_of_denying_it(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    run(&mut f, delete_pod());
    assert!(
        modal_open(&mut f),
        "the guard's confirmation opens a dialog"
    );
    assert!(bounds(&mut f.vcx, "dialog-modal").is_some());
    assert_eq!(*f.deletes.lock(), 0);

    f.vcx.simulate_keystrokes("enter");
    f.vcx.run_until_parked();
    assert_eq!(*f.deletes.lock(), 1, "confirming dispatches it again");
    assert!(!modal_open(&mut f));
    assert!(
        toasts(&mut f)
            .iter()
            .any(|(level, m)| *level == ToastLevel::Success && m == "Deleted")
    );
}

fn node_shell() -> Command {
    Command::NodeShell {
        target: ResourceRef::cluster_scoped(id(PROD), Gvk::new("", "v1", "Node"), "worker-1"),
    }
}

#[gpui::test]
fn a_node_shell_asks_with_the_node_and_the_image_named_and_opens_only_when_confirmed(
    cx: &mut TestAppContext,
) {
    let mut f = fixture(cx);
    run(&mut f, node_shell());
    let layer = f.vcx.update(|_, cx| f.ws.read(cx).modal_layer().clone());
    let dialog = f
        .vcx
        .update(|_, cx| layer.read(cx).active_modal::<crate::modal::DialogModal>())
        .expect("the guard's confirmation opens a dialog");
    let (title, message) = f.vcx.update(|_, cx| {
        let dialog = dialog.read(cx);
        (
            dialog.title().to_string(),
            dialog
                .message_text()
                .map(ToString::to_string)
                .unwrap_or_default(),
        )
    });
    assert_eq!(title, "Open a shell on the node?");
    assert!(message.contains("worker-1"), "{message}");
    assert!(message.contains("busybox:1.37"), "{message}");
    assert!(message.contains("privileged"), "{message}");
    assert_eq!(*f.node_shells.lock(), 0, "nothing runs before the answer");

    // Cancelling opens nothing and is audited.
    f.vcx.simulate_keystrokes("escape");
    f.vcx.run_until_parked();
    assert_eq!(*f.node_shells.lock(), 0);
    assert!(f.state.audit_log().iter().any(|r| r.outcome
        == oxikube_domain::audit::AuditOutcome::Cancelled
        && &*r.cmd == "node::Shell"));

    run(&mut f, node_shell());
    f.vcx.simulate_keystrokes("enter");
    f.vcx.run_until_parked();
    assert_eq!(*f.node_shells.lock(), 1, "confirming dispatches it again");
}

#[gpui::test]
fn a_node_shell_on_a_read_only_cluster_is_refused_with_a_toast_and_no_dialog(
    cx: &mut TestAppContext,
) {
    let mut f = fixture(cx);
    f.manager.set_read_only(&id(PROD), true).unwrap();
    f.vcx.run_until_parked();
    run(&mut f, node_shell());
    assert!(!modal_open(&mut f), "refused before it asks");
    assert_eq!(*f.node_shells.lock(), 0);
    let shown = toasts(&mut f);
    assert!(
        shown.iter().any(|(_, m)| m.contains("read-only")),
        "{shown:?}"
    );
}

#[gpui::test]
fn a_debug_container_is_confirmed_in_its_own_words_not_as_turning_something_off(
    cx: &mut TestAppContext,
) {
    use crate::modal::DialogModal;

    let mut f = fixture(cx);
    run(
        &mut f,
        Command::PodDebug {
            target: ResourceRef::namespaced(
                id(PROD),
                Gvk::new("", "v1", "Pod"),
                "default",
                "web-0",
            ),
            image: "busybox".into(),
            target_container: Some("app".into()),
            command: Vec::new(),
            name: None,
        },
    );
    assert!(modal_open(&mut f), "the guard asks first");
    assert_eq!(*f.debugs.lock(), 0, "nothing ran before the answer");
    let layer = f.vcx.update(|_, cx| f.ws.read(cx).modal_layer().clone());
    let (title, message) = f.vcx.update(|_, cx| {
        let dialog = layer
            .read(cx)
            .active_modal::<DialogModal>()
            .expect("a dialog");
        let dialog = dialog.read(cx);
        (
            dialog.title().to_string(),
            dialog
                .message_text()
                .map(ToString::to_string)
                .unwrap_or_default(),
        )
    });
    assert_eq!(title, "Add a debug container?");
    for part in ["busybox", "container app", "web-0", "cannot be removed"] {
        assert!(message.contains(part), "{part}: {message}");
    }
    assert!(bounds(&mut f.vcx, "dialog-modal").is_some());

    f.vcx.simulate_keystrokes("enter");
    f.vcx.run_until_parked();
    assert_eq!(*f.debugs.lock(), 1, "confirming runs it");
    assert!(
        toasts(&mut f)
            .iter()
            .any(|(level, m)| *level == ToastLevel::Success && m.contains("debugger-x1y2z"))
    );
}

#[gpui::test]
fn lifting_read_only_on_a_production_cluster_confirms_first(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    run(
        &mut f,
        Command::ClusterApplyPreset {
            cluster: id(PROD),
            preset: ClusterPreset::Prod,
        },
    );
    assert!(f.manager.get(&id(PROD)).unwrap().read_only());
    assert!(bounds(&mut f.vcx, "status-cluster-read-only").is_some());

    let lift = Command::ClusterToggleReadOnly {
        cluster: id(PROD),
        read_only: Some(false),
    };
    run(&mut f, lift.clone());
    assert!(modal_open(&mut f), "production asks first");
    assert!(f.manager.get(&id(PROD)).unwrap().read_only(), "not yet");

    // Cancelling keeps it on and the cancellation is audited.
    f.vcx.simulate_keystrokes("escape");
    f.vcx.run_until_parked();
    assert!(f.manager.get(&id(PROD)).unwrap().read_only());
    assert!(
        f.state
            .audit_log()
            .iter()
            .any(|r| r.outcome == oxikube_domain::audit::AuditOutcome::Cancelled)
    );

    run(&mut f, lift);
    f.vcx.simulate_keystrokes("enter");
    f.vcx.run_until_parked();
    assert!(!f.manager.get(&id(PROD)).unwrap().read_only());
    assert!(
        bounds(&mut f.vcx, "status-cluster-read-only").is_none(),
        "the badge goes away with the flag"
    );
}

#[gpui::test]
fn denial_toasts_name_the_cluster_and_use_one_key_per_cluster(cx: &mut TestAppContext) {
    use crate::cluster::denial_toast;
    use oxikube_app::DispatchError;
    let mut f = fixture(cx);
    let read_only = DispatchError::ReadOnly {
        cluster: id(PROD),
        context: ContextName::new("prod-eu"),
    };
    let other = DispatchError::NoSession(id(PROD));
    f.vcx.update(|_, cx| {
        f.ws.update(cx, |ws, cx| {
            ws.show_toast(denial_toast(&read_only), cx);
            ws.show_toast(denial_toast(&read_only), cx);
            ws.show_toast(denial_toast(&other), cx);
        });
    });
    f.vcx.run_until_parked();
    let layer = f.vcx.update(|_, cx| f.ws.read(cx).toast_layer().clone());
    let shown = f.vcx.update(|_, cx| layer.read(cx).visible());
    assert_eq!(shown.len(), 2, "the two read-only toasts share a key");
    assert_eq!(shown[0].level, ToastLevel::Warning);
    assert!(shown[0].message.contains("prod-eu"));
    assert_eq!(
        shown[0].key.as_deref(),
        Some(format!("read-only:{}", id(PROD)).as_str())
    );
    assert_eq!(shown[1].level, ToastLevel::Error);
    assert!(shown[1].key.is_none());
}
