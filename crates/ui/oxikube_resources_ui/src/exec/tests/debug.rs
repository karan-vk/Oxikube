//! The debug dialog (E09-S10): "Debug" on a pod's row, key and detail header opens it with the
//! defaults (busybox, the pod's default container, `sh`), it checks the fields before sending
//! anything, submitting runs `pod::Debug` through the bus (guarded and audited), and a refusal
//! stays on screen with the API server's message.

use gpui::{Entity, TestAppContext};
use oxikube_app::exec::DEFAULT_CONTAINER_ANNOTATION;
use oxikube_domain::Resource;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::command::CommandId;
use oxikube_ports::ClusterPrefs;
use serde_json::json;

use super::pod_with;
use crate::detail::tests::fixture::{Detail, pod_ref};
use crate::exec::{DebugDialog, DebugStage};
use crate::table::tests::fixture::{Fixture, cluster};

/// The debug dialog open in the cluster tab's workspace, if any.
fn dialog(f: &mut Fixture) -> Option<Entity<DebugDialog>> {
    let tabs = f.tabs.clone();
    f.vcx.update(|_, cx| {
        let tab = tabs.read(cx).tab(&cluster())?.clone();
        let layer = tab.read(cx).workspace().read(cx).modal_layer().clone();
        layer.read(cx).active_modal::<DebugDialog>()
    })
}

/// Opens the pods table over `pod` and presses `shift-d` on its row. The pod is served to the
/// flow's read as well (the table's feed and the dialog's defaults are separate reads).
fn press_debug(f: &mut Fixture, pod: Resource) -> Entity<DebugDialog> {
    f.connect_with([pod.clone()]);
    let table = f.open_pods();
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.ports().resources.script().get.push_ok(pod);
    f.dispatcher.clear();
    f.keys(&table, "shift-d");
    f.settle();
    dialog(f).expect("the debug dialog is open")
}

fn fill(f: &mut Fixture, dialog: &Entity<DebugDialog>, image: &str, command: &str, name: &str) {
    f.vcx.update(|window, cx| {
        dialog.update(cx, |d, cx| d.fill(image, command, name, window, cx));
    });
}

fn submit(f: &mut Fixture, dialog: &Entity<DebugDialog>) {
    f.vcx
        .update(|_, cx| dialog.update(cx, |d, cx| d.submit(cx)));
    f.settle();
}

fn debug_audit(f: &Fixture) -> Vec<(AuditOutcome, String)> {
    f.state
        .audit_log()
        .iter()
        .filter(|r| &*r.cmd == "pod::Debug")
        .map(|r| (r.outcome, r.detail.as_deref().unwrap_or("").to_owned()))
        .collect()
}

fn annotated(pod: Resource, default: &str) -> Resource {
    let mut json = pod.json;
    json["metadata"]["annotations"] = json!({ DEFAULT_CONTAINER_ANNOTATION: default });
    Resource::from_json(json).expect("a pod")
}

#[gpui::test]
fn the_dialog_starts_with_busybox_sh_and_the_pods_default_container(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    let dialog = press_debug(
        &mut f,
        annotated(pod_with("x", "web-0", &["app", "proxy"]), "proxy"),
    );
    let (image, command, name, target, targets) = f.vcx.update(|_, cx| {
        let d = dialog.read(cx);
        (
            d.defaults().image.clone(),
            d.defaults().command.clone(),
            d.request(cx).unwrap(),
            d.target_index(),
            d.defaults()
                .targets
                .iter()
                .map(|c| c.name.to_string())
                .collect::<Vec<_>>(),
        )
    });
    assert_eq!((image.as_str(), command.as_str()), ("busybox", "sh"));
    assert_eq!(targets, ["app", "proxy"]);
    assert_eq!(target, 1, "the default-container annotation");
    assert_eq!(name.image, "busybox");
    assert_eq!(name.target_container.as_deref(), Some("proxy"));
    assert_eq!(name.command, ["sh"]);
    assert_eq!(name.name, None, "the name is generated unless typed");
    assert!(f.dispatcher.sent().is_empty(), "opening sends nothing");

    // Everything is on screen, with the warning that the container is permanent.
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    for part in [
        "debug-dialog",
        "debug-warning",
        "debug-image",
        "debug-target-0",
        "debug-target-1",
        "debug-command",
        "debug-name",
        "debug-confirm",
    ] {
        assert!(f.vcx.debug_bounds(part).is_some(), "{part} is drawn");
    }
    assert!(
        crate::exec::PERMANENCE_NOTE.contains("cannot be removed"),
        "the dialog says so"
    );
}

#[gpui::test]
fn the_target_can_be_chosen_and_the_fields_are_what_is_sent(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    let dialog = press_debug(&mut f, pod_with("x", "web-0", &["app", "proxy"]));
    f.vcx
        .update(|_, cx| dialog.update(cx, |d, cx| d.select_target(1, cx)));
    fill(
        &mut f,
        &dialog,
        "nicolaka/netshoot",
        r#"bash -c "ss -tlnp""#,
        "netshoot",
    );
    let request = f.vcx.update(|_, cx| dialog.read(cx).request(cx)).unwrap();
    assert_eq!(request.image, "nicolaka/netshoot");
    assert_eq!(request.target_container.as_deref(), Some("proxy"));
    assert_eq!(request.command, ["bash", "-c", "ss -tlnp"]);
    assert_eq!(request.name.as_deref(), Some("netshoot"));
}

#[gpui::test]
fn bad_fields_are_shown_under_the_form_and_nothing_is_sent(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    let dialog = press_debug(&mut f, pod_with("x", "web-0", &["app"]));
    for (image, command, name, says) in [
        ("", "sh", "", "image"),
        ("two words", "sh", "", "image"),
        ("busybox", "sh -c 'oops", "", "quote"),
        ("busybox", "sh", "Not Valid", "name"),
    ] {
        fill(&mut f, &dialog, image, command, name);
        submit(&mut f, &dialog);
        let (stage, error) = f.vcx.update(|_, cx| {
            (
                dialog.read(cx).stage(),
                dialog.read(cx).error().map(str::to_owned),
            )
        });
        assert_eq!(stage, DebugStage::Editing, "{image:?} {command:?} {name:?}");
        let error = error.unwrap_or_else(|| panic!("an error for {image:?} {command:?}"));
        assert!(error.contains(says), "{error}");
    }
    assert!(f.state.audit_log().is_empty(), "nothing reached the bus");
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(f.vcx.debug_bounds("debug-error").is_some());
}

#[gpui::test]
fn submitting_runs_pod_debug_through_the_guard_and_closes_the_dialog(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    let dialog = press_debug(&mut f, pod_with("x", "web-0", &["app"]));
    fill(&mut f, &dialog, "nicolaka/netshoot", "bash", "");
    submit(&mut f, &dialog);
    assert!(super::debug::dialog(&mut f).is_none(), "the dialog closed");
    assert_eq!(
        debug_audit(&f),
        [(
            AuditOutcome::Succeeded,
            "session=debug image=nicolaka/netshoot target=app program=bash".to_owned()
        )],
        "one guarded, audited command"
    );
    let toasts = crate::actions::tests::toasts(&mut f);
    assert!(
        toasts.iter().any(|t| t.message.contains("debugger-ab12c")),
        "{toasts:?}"
    );
}

#[gpui::test]
fn a_refused_container_stays_in_the_dialog_with_the_servers_message(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    let dialog = press_debug(&mut f, pod_with("x", "web-0", &["app"]));
    fill(&mut f, &dialog, "reject/me", "sh", "");
    submit(&mut f, &dialog);
    let (stage, error) = f.vcx.update(|_, cx| {
        (
            dialog.read(cx).stage(),
            dialog.read(cx).error().map(str::to_owned),
        )
    });
    assert_eq!(stage, DebugStage::Editing, "back to the form");
    assert!(error.unwrap().contains("PodSecurity"));
    assert_eq!(debug_audit(&f).len(), 1);
    assert_eq!(debug_audit(&f)[0].0, AuditOutcome::Failed);

    // Fixing the image and trying again works.
    fill(&mut f, &dialog, "busybox", "sh", "");
    submit(&mut f, &dialog);
    assert!(super::debug::dialog(&mut f).is_none());
    assert_eq!(debug_audit(&f).len(), 2);
}

#[gpui::test]
fn escape_cancels_and_sends_nothing(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    press_debug(&mut f, pod_with("x", "web-0", &["app"]));
    f.vcx.simulate_keystrokes("escape");
    f.settle();
    assert!(super::debug::dialog(&mut f).is_none());
    assert!(f.state.audit_log().is_empty(), "no command, no record");
}

#[gpui::test]
fn the_menu_item_and_the_read_only_block(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    f.connect_with([pod_with("x", "web-0", &["app"])]);
    let table = f.open_pods();
    f.sessions.set_read_only(&cluster(), true).unwrap();
    let entries = f.vcx.update(|_, cx| table.read(cx).action_entries(cx));
    let debug = entries
        .iter()
        .find(|e| e.command() == CommandId::POD_DEBUG)
        .expect("Debug is offered");
    assert!(!debug.is_enabled(), "patching a pod is a mutation");
    assert!(
        debug.reason().unwrap().contains("read-only"),
        "{:?}",
        debug.reason()
    );
    // The key says why instead of opening anything, and even an exec-in-read-only cluster keeps
    // it blocked: the setting is for shells, not for changing pods.
    f.sessions.set_prefs_table(
        oxikube_ports::ClusterPrefsTable::new(ClusterPrefs::default()).with_cluster(
            cluster(),
            ClusterPrefs {
                read_only: true,
                exec_in_read_only: true,
                ..ClusterPrefs::default()
            },
        ),
    );
    f.settle();
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.keys(&table, "shift-d");
    f.settle();
    assert!(super::debug::dialog(&mut f).is_none());
    assert!(f.state.audit_log().is_empty());
}

#[gpui::test]
fn the_pod_detail_header_has_a_debug_button(cx: &mut TestAppContext) {
    let mut d = Detail::with_exec(
        cx,
        [
            crate::detail::tests::fixture::web_pod(),
            crate::detail::tests::fixture::web_replicaset(),
        ],
    );
    d.open(&pod_ref("web-0"));
    assert!(d.shown("detail-debug"));
    d.f.ports()
        .resources
        .script()
        .get
        .push_ok(crate::detail::tests::fixture::web_pod());
    d.click("detail-debug");
    d.settle();
    assert!(
        super::debug::dialog(&mut d.f).is_some(),
        "the dialog opens in the tab's workspace"
    );

    // A read-only cluster disables it, with the reason.
    d.f.sessions.set_read_only(&cluster(), true).unwrap();
    d.settle();
    d.f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(d.shown("detail-debug"), "still there, greyed out");
}
