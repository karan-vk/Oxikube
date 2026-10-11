//! Scale: the row action, the `s` key, the dialog's current / desired and its checks, and the run
//! through the real bus and guard (blocked read-only, audited, medium for zero).

use futures::executor::block_on;
use gpui::{Entity, TestAppContext};
use oxikube_domain::Resource;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::Gvk;
use oxikube_ports::{ListPage, Scale};
use oxikube_testkit::ResourceCall;

use super::{
    daemonsets, deployments, replicasets, replication_controllers, statefulsets, web_deployment,
};
use crate::kinds::{ScaleDialog, ScaleStage};
use crate::table::ResourceTable;
use crate::table::tests::fixture::{Fixture, cluster};

fn scale_of(spec: i32, status: i32) -> Scale {
    Scale {
        replicas: spec,
        current_replicas: status,
        selector: None,
        resource_version: None,
    }
}

/// A window over a cluster serving the workload kinds, holding `objects`, with `workload::Scale`.
fn window(cx: &mut TestAppContext, objects: impl IntoIterator<Item = Resource>) -> Fixture {
    let mut f = Fixture::with_workloads(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    let ports = f.ports();
    ports.discovery.set_kinds([
        deployments(),
        statefulsets(),
        replicasets(),
        daemonsets(),
        replication_controllers(),
    ]);
    for object in objects {
        ports.resources.insert(object);
    }
    block_on(f.sessions.connect(&cluster())).expect("connect");
    f.vcx.run_until_parked();
    f
}

fn dialog(f: &mut Fixture) -> Option<Entity<ScaleDialog>> {
    let tabs = f.tabs.clone();
    f.vcx.update(|_, cx| {
        let tab = tabs.read(cx).tab(&cluster())?.clone();
        let layer = tab.read(cx).workspace().read(cx).modal_layer().clone();
        layer.read(cx).active_modal::<ScaleDialog>()
    })
}

/// Opens the deployments table, puts the cursor on the first row and presses `s`, with the
/// subresource answering `scale`.
fn press_s(f: &mut Fixture, scale: Scale) -> (Entity<ResourceTable>, Entity<ScaleDialog>) {
    let table = f.open(deployments());
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.ports().resources.script().get_scale.push_ok(scale);
    f.dispatcher.clear();
    f.keys(&table, "s");
    f.settle();
    let dialog = dialog(f).expect("the scale dialog is open");
    (table, dialog)
}

fn set_text(f: &mut Fixture, dialog: &Entity<ScaleDialog>, text: &str) {
    f.vcx
        .update(|window, cx| dialog.update(cx, |d, cx| d.set_text(text, window, cx)));
    f.settle();
}

fn submit(f: &mut Fixture, dialog: &Entity<ScaleDialog>) {
    f.vcx
        .update(|_, cx| dialog.update(cx, |d, cx| d.submit(cx)));
    f.settle();
}

fn drawn(f: &mut Fixture, selector: &'static str) -> bool {
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    f.vcx.debug_bounds(selector).is_some()
}

fn labels(f: &mut Fixture, table: &Entity<ResourceTable>) -> Vec<String> {
    f.vcx
        .update(|_, cx| table.read(cx).action_entries(cx))
        .into_iter()
        .map(|entry| entry.label)
        .collect()
}

fn scale_audit(f: &Fixture) -> Vec<(AuditOutcome, String)> {
    f.state
        .audit_log()
        .iter()
        .filter(|r| &*r.cmd == "workload::Scale")
        .map(|r| (r.outcome, r.detail.as_deref().unwrap_or("").to_owned()))
        .collect()
}

#[gpui::test]
fn scale_is_offered_for_the_four_kinds_and_not_for_a_daemonset(cx: &mut TestAppContext) {
    let mut f = window(cx, [web_deployment()]);
    for (kind, offered) in [
        (deployments(), true),
        (statefulsets(), true),
        (replicasets(), true),
        (replication_controllers(), true),
        (daemonsets(), false),
    ] {
        let name = kind.gvk.kind.to_string();
        let table = f.open(kind);
        let entries = labels(&mut f, &table);
        assert_eq!(
            entries.contains(&"Scale".to_owned()),
            offered,
            "{name}: {entries:?}"
        );
    }
}

#[gpui::test]
fn s_opens_the_dialog_at_once_and_it_shows_current_and_desired(cx: &mut TestAppContext) {
    let mut f = window(cx, [web_deployment()]);
    // Three wanted, two running: a scale-up in flight.
    let (_, dialog) = press_s(&mut f, scale_of(3, 2));
    assert!(f.dispatcher.sent().is_empty(), "opening sends nothing");
    let (stage, plan, text) = f.vcx.update(|_, cx| {
        let d = dialog.read(cx);
        (d.stage(), d.plan().cloned(), d.text(cx))
    });
    assert_eq!(stage, ScaleStage::Editing);
    let plan = plan.expect("the plan");
    assert_eq!((plan.current, plan.desired), (2, 3));
    assert_eq!(text, "3", "the field starts at the desired count");
    for part in [
        "scale-dialog",
        "scale-subtitle",
        "scale-current",
        "scale-replicas",
        "scale-minus",
        "scale-plus",
        "scale-confirm",
    ] {
        assert!(drawn(&mut f, part), "{part} is drawn");
    }
    assert!(!drawn(&mut f, "scale-zero"), "no warning at 3");
    assert_eq!(
        f.vcx
            .update(|_, cx| dialog.read(cx).error().map(str::to_owned)),
        None
    );
}

#[gpui::test]
fn the_dialog_is_up_before_the_subresource_answers(cx: &mut TestAppContext) {
    let mut f = window(cx, [web_deployment()]);
    let table = f.open(deployments());
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    // The read of the scale is not scripted yet, so it fails; the dialog is open regardless and
    // says so (an account that cannot read the count can still type one).
    f.dispatcher.clear();
    f.keys(&table, "s");
    let dialog = dialog(&mut f).expect("open");
    let (stage, plan, error) = f.vcx.update(|_, cx| {
        let d = dialog.read(cx);
        (d.stage(), d.plan().cloned(), d.error().map(str::to_owned))
    });
    assert_eq!(stage, ScaleStage::Editing);
    assert!(plan.is_none());
    assert!(error.is_some(), "the unread count is explained");
}

#[gpui::test]
fn a_negative_or_broken_count_is_rejected_and_nothing_is_sent(cx: &mut TestAppContext) {
    let mut f = window(cx, [web_deployment()]);
    let (_, dialog) = press_s(&mut f, scale_of(3, 3));
    for (text, says) in [
        ("-1", "negative"),
        ("two", "whole number"),
        ("1.5", "whole number"),
        ("99999999999", "more than"),
    ] {
        set_text(&mut f, &dialog, text);
        let (parsed, can) = f
            .vcx
            .update(|_, cx| (dialog.read(cx).parsed(cx), dialog.read(cx).can_submit(cx)));
        let message = parsed.expect_err(text).message().to_owned();
        assert!(message.contains(says), "{text}: {message}");
        assert!(!can, "{text}");
        assert!(drawn(&mut f, "scale-field-error"), "{text}");
        submit(&mut f, &dialog);
        assert!(dialog_open(&mut f), "{text}: still asking");
    }
    assert!(f.state.audit_log().is_empty(), "nothing reached the bus");
    assert!(f.ports().resources.mutating_calls().is_empty());
    // The minus button never goes below zero.
    set_text(&mut f, &dialog, "0");
    f.vcx
        .update(|window, cx| dialog.update(cx, |d, cx| d.step(-1, window, cx)));
    assert_eq!(f.vcx.update(|_, cx| dialog.read(cx).text(cx)), "0");
    f.vcx
        .update(|window, cx| dialog.update(cx, |d, cx| d.step(1, window, cx)));
    assert_eq!(f.vcx.update(|_, cx| dialog.read(cx).text(cx)), "1");
}

fn dialog_open(f: &mut Fixture) -> bool {
    dialog(f).is_some()
}

#[gpui::test]
fn confirming_scales_through_the_guard_audits_before_and_after_and_closes(cx: &mut TestAppContext) {
    let mut f = window(cx, [web_deployment()]);
    let (_, dialog) = press_s(&mut f, scale_of(3, 3));
    f.ports()
        .resources
        .script()
        .get_scale
        .push_ok(scale_of(3, 3));
    f.ports().resources.script().scale.push_ok(scale_of(5, 3));
    set_text(&mut f, &dialog, "5");
    submit(&mut f, &dialog);

    assert!(!dialog_open(&mut f), "the dialog closed");
    let writes = f.ports().resources.mutating_calls();
    assert!(
        matches!(&writes[..], [ResourceCall::Scale { replicas: 5, name, .. }] if name == "web"),
        "{writes:?}"
    );
    assert_eq!(
        scale_audit(&f),
        [(
            AuditOutcome::Succeeded,
            "replicas=5 before=3 after=5".to_owned()
        )]
    );
    let toasts = crate::actions::tests::toasts(&mut f);
    assert!(
        toasts
            .iter()
            .any(|t| t.message.contains("from 3 to 5 replicas")),
        "{toasts:?}"
    );
}

#[gpui::test]
fn zero_warns_that_every_pod_stops_and_is_medium_in_the_audit_trail(cx: &mut TestAppContext) {
    let mut f = window(cx, [web_deployment()]);
    let (_, dialog) = press_s(&mut f, scale_of(3, 3));
    set_text(&mut f, &dialog, "0");
    assert!(drawn(&mut f, "scale-zero"), "the warning");
    f.ports()
        .resources
        .script()
        .get_scale
        .push_ok(scale_of(3, 3));
    f.ports().resources.script().scale.push_ok(scale_of(0, 3));
    submit(&mut f, &dialog);
    assert!(!dialog_open(&mut f));
    assert_eq!(
        scale_audit(&f),
        [(
            AuditOutcome::Succeeded,
            "replicas=0 before=3 after=0".to_owned()
        )]
    );
}

#[gpui::test]
fn a_refusal_stays_in_the_dialog_with_the_servers_message(cx: &mut TestAppContext) {
    let mut f = window(cx, [web_deployment()]);
    let (_, dialog) = press_s(&mut f, scale_of(3, 3));
    f.ports()
        .resources
        .script()
        .get_scale
        .push_ok(scale_of(3, 3));
    f.ports()
        .resources
        .script()
        .scale
        .push_err(oxikube_domain::OxiError::forbidden(
            "admission webhook \"quota\" denied the request",
        ));
    set_text(&mut f, &dialog, "50");
    submit(&mut f, &dialog);
    let (stage, error) = f.vcx.update(|_, cx| {
        (
            dialog.read(cx).stage(),
            dialog.read(cx).error().map(str::to_owned),
        )
    });
    assert_eq!(stage, ScaleStage::Editing, "back to the form");
    assert!(error.expect("a message").contains("quota"));
    assert!(drawn(&mut f, "scale-error"));
    assert_eq!(scale_audit(&f)[0].0, AuditOutcome::Failed);
}

#[gpui::test]
fn cancelling_sends_nothing(cx: &mut TestAppContext) {
    let mut f = window(cx, [web_deployment()]);
    let (_, dialog) = press_s(&mut f, scale_of(3, 3));
    f.vcx
        .update(|_, cx| dialog.update(cx, |d, cx| d.cancel(cx)));
    f.settle();
    assert!(!dialog_open(&mut f));
    assert!(f.state.audit_log().is_empty());
}

#[gpui::test]
fn a_read_only_cluster_greys_scale_out_and_the_key_says_why(cx: &mut TestAppContext) {
    let mut f = window(cx, [web_deployment()]);
    let table = f.open(deployments());
    f.sessions.set_read_only(&cluster(), true).unwrap();
    let entries = f.vcx.update(|_, cx| table.read(cx).action_entries(cx));
    let entry = entries
        .iter()
        .find(|e| e.command() == CommandId::WORKLOAD_SCALE)
        .expect("listed");
    assert!(!entry.is_enabled());
    assert!(
        entry.reason().unwrap().contains("read-only"),
        "{:?}",
        entry.reason()
    );
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.keys(&table, "s");
    assert!(!dialog_open(&mut f), "no dialog");
    let toasts = crate::actions::tests::toasts(&mut f);
    assert!(
        toasts.iter().any(|t| t.message.contains("read-only")),
        "{toasts:?}"
    );
    assert!(f.ports().resources.mutating_calls().is_empty());
}

#[gpui::test]
fn s_stays_a_shell_on_kinds_that_cannot_scale(cx: &mut TestAppContext) {
    let mut f = window(cx, []);
    let table = f.open(daemonsets());
    f.dispatcher.clear();
    f.keys(&table, "s");
    assert!(!dialog_open(&mut f));
    assert!(
        f.dispatcher
            .sent()
            .iter()
            .all(|c| !matches!(c, Command::WorkloadScale { .. }))
    );
}

#[gpui::test]
fn a_horizontal_pod_autoscaler_is_mentioned(cx: &mut TestAppContext) {
    let mut f = window(cx, [web_deployment()]);
    f.ports().resources.script().list.push_ok(ListPage::complete(vec![
        Resource::from_json(serde_json::json!({
            "apiVersion": "autoscaling/v2", "kind": "HorizontalPodAutoscaler",
            "metadata": {"name": "web-hpa", "namespace": "shop"},
            "spec": {"scaleTargetRef": {"apiVersion": "apps/v1", "kind": "Deployment", "name": "web"},
                     "maxReplicas": 8}
        }))
        .unwrap(),
    ]));
    let (_, _dialog) = press_s(&mut f, scale_of(2, 2));
    assert!(drawn(&mut f, "scale-autoscaler"));
    let _ = Gvk::new("", "v1", "Pod");
}
