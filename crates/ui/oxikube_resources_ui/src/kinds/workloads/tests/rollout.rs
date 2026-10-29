//! Restart and Set image: the row actions, the `r` and `i` keys, the dialog's form, diff and
//! checks, and the run through the real bus and guard (blocked read-only, audited), plus the
//! detail header's buttons and the palette.

use futures::executor::block_on;
use gpui::{Entity, TestAppContext};
use oxikube_domain::Resource;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_ports::PatchKind;
use oxikube_testkit::ResourceCall;
use serde_json::json;

use super::{
    daemonsets, deployments, kind, replicasets, replication_controllers, statefulsets,
    web_deployment, web_ref,
};
use crate::detail::tests::fixture::Detail;
use crate::kinds::{SetImageDialog, SetImageStage};
use crate::table::ResourceTable;
use crate::table::tests::fixture::{Fixture, cluster};

/// A Deployment `shop/web` with an init container `migrate`, `web` and `sidecar`.
fn multi_container() -> Resource {
    let mut deployment = web_deployment().to_value();
    deployment["spec"]["template"]["spec"] = json!({
        "initContainers": [{"name": "migrate", "image": "shop/migrate:1"}],
        "containers": [
            {"name": "web", "image": "shop/web:2.1", "ports": [{"containerPort": 8080}]},
            {"name": "sidecar", "image": "envoy:1"},
        ],
    });
    Resource::from_json(deployment).expect("a deployment")
}

fn daemonset() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "apps/v1", "kind": "DaemonSet",
        "metadata": {"name": "agent", "namespace": "shop", "uid": "ds-uid", "resourceVersion": "4",
                     "creationTimestamp": "2026-01-01T00:00:00Z"},
        "spec": {"selector": {"matchLabels": {"app": "agent"}},
                 "template": {"metadata": {"labels": {"app": "agent"}},
                              "spec": {"containers": [{"name": "agent", "image": "agent:1"}]}}},
        "status": {"desiredNumberScheduled": 1, "numberReady": 1}
    }))
    .expect("a daemonset")
}

fn window(cx: &mut TestAppContext, objects: impl IntoIterator<Item = Resource>) -> Fixture {
    let mut f = Fixture::with_workloads(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    f.ports().discovery.set_kinds([
        deployments(),
        statefulsets(),
        replicasets(),
        daemonsets(),
        replication_controllers(),
        kind("", "Pod", "pods"),
    ]);
    for object in objects {
        f.ports().resources.insert(object);
    }
    block_on(f.sessions.connect(&cluster())).expect("connect");
    f.vcx.run_until_parked();
    f
}

fn dialog(f: &mut Fixture) -> Option<Entity<SetImageDialog>> {
    let tabs = f.tabs.clone();
    f.vcx.update(|_, cx| {
        let tab = tabs.read(cx).tab(&cluster())?.clone();
        let layer = tab.read(cx).workspace().read(cx).modal_layer().clone();
        layer.read(cx).active_modal::<SetImageDialog>()
    })
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

/// Opens the deployments table with the cursor on the first row and presses `i`.
fn press_i(f: &mut Fixture) -> Entity<SetImageDialog> {
    let table = f.open(deployments());
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.dispatcher.clear();
    f.keys(&table, "i");
    f.settle();
    dialog(f).expect("the set-image dialog is open")
}

fn set_text(f: &mut Fixture, dialog: &Entity<SetImageDialog>, container: &str, text: &str) {
    f.vcx.update(|window, cx| {
        dialog.update(cx, |d, cx| d.set_text(container, text, window, cx));
    });
    f.settle();
}

fn submit(f: &mut Fixture, dialog: &Entity<SetImageDialog>) {
    f.vcx
        .update(|_, cx| dialog.update(cx, |d, cx| d.submit(cx)));
    f.settle();
}

fn audit(f: &Fixture, cmd: &str) -> Vec<(AuditOutcome, String)> {
    f.state
        .audit_log()
        .iter()
        // The commit's records: the guard's dry run before it asks (E19-S07) is its own.
        .filter(|r| &*r.cmd == cmd && !r.dry_run)
        .map(|r| (r.outcome, r.detail.as_deref().unwrap_or("").to_owned()))
        .collect()
}

#[gpui::test]
fn restart_and_set_image_are_offered_for_deployments_statefulsets_and_daemonsets_only(
    cx: &mut TestAppContext,
) {
    let mut f = window(cx, [web_deployment()]);
    for (kind, offered) in [
        (deployments(), true),
        (statefulsets(), true),
        (daemonsets(), true),
        (replicasets(), false),
        (replication_controllers(), false),
    ] {
        let name = kind.gvk.kind.to_string();
        let table = f.open(kind);
        let entries = labels(&mut f, &table);
        for label in ["Restart", "Set image"] {
            assert_eq!(
                entries.contains(&label.to_owned()),
                offered,
                "{name}: {entries:?}"
            );
        }
    }
}

#[gpui::test]
fn r_sends_workload_restart_for_the_cursor_row_and_only_on_those_kinds(cx: &mut TestAppContext) {
    let mut f = window(cx, [web_deployment(), daemonset()]);
    for kind in [deployments(), daemonsets()] {
        let table = f.open(kind);
        f.update(&table, |t, cx| t.move_cursor(1, false, cx));
        f.dispatcher.clear();
        f.keys(&table, "r");
        let sent = f.dispatcher.sent();
        assert!(
            matches!(&sent[..], [Command::WorkloadRestart { target }]
                if matches!(&*target.name, "web" | "agent")),
            "{sent:?}"
        );
    }
    // A ReplicaSet's `r` is not bound: nothing is sent.
    let table = f.open(replicasets());
    f.dispatcher.clear();
    f.keys(&table, "r");
    assert!(f.dispatcher.sent().is_empty());
}

#[gpui::test]
fn a_read_only_cluster_greys_both_out_and_the_keys_say_why(cx: &mut TestAppContext) {
    let mut f = window(cx, [web_deployment()]);
    let table = f.open(deployments());
    f.sessions.set_read_only(&cluster(), true).unwrap();
    let entries = f.vcx.update(|_, cx| table.read(cx).action_entries(cx));
    for command in [CommandId::WORKLOAD_RESTART, CommandId::WORKLOAD_SET_IMAGE] {
        let entry = entries
            .iter()
            .find(|e| e.command() == command)
            .expect("listed");
        assert!(!entry.is_enabled(), "{command}");
        assert!(entry.reason().unwrap().contains("read-only"), "{command}");
    }
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.dispatcher.clear();
    f.keys(&table, "r");
    f.keys(&table, "i");
    assert!(dialog(&mut f).is_none(), "no dialog");
    assert!(f.dispatcher.sent().is_empty(), "no restart sent");
    let toasts = crate::actions::tests::toasts(&mut f);
    assert!(
        toasts.iter().any(|t| t.message.contains("read-only")),
        "{toasts:?}"
    );
    assert!(f.ports().resources.mutating_calls().is_empty());
}

#[gpui::test]
fn i_opens_the_dialog_at_once_with_a_field_per_container_init_first(cx: &mut TestAppContext) {
    let mut f = window(cx, [multi_container()]);
    let dialog = press_i(&mut f);
    assert!(f.dispatcher.sent().is_empty(), "opening sends nothing");
    let (stage, rows) = f.vcx.update(|_, cx| {
        let d = dialog.read(cx);
        let rows: Vec<_> = d
            .fields()
            .iter()
            .map(|f| (f.name.to_string(), f.init, d.text(&f.name, cx).unwrap()))
            .collect();
        (d.stage(), rows)
    });
    assert_eq!(stage, SetImageStage::Editing);
    assert_eq!(
        rows,
        [
            ("migrate".to_owned(), true, "shop/migrate:1".to_owned()),
            ("web".to_owned(), false, "shop/web:2.1".to_owned()),
            ("sidecar".to_owned(), false, "envoy:1".to_owned()),
        ]
    );
    for part in [
        "setimage-dialog",
        "setimage-subtitle",
        "setimage-row-0",
        "setimage-row-2",
        "setimage-diff",
        "setimage-no-changes",
        "setimage-confirm",
    ] {
        assert!(drawn(&mut f, part), "{part} is drawn");
    }
    // Nothing changed yet: nothing to send.
    assert!(!f.vcx.update(|_, cx| dialog.read(cx).can_submit(cx)));
}

#[gpui::test]
fn editing_a_field_shows_the_diff_old_to_new(cx: &mut TestAppContext) {
    let mut f = window(cx, [multi_container()]);
    let dialog = press_i(&mut f);
    set_text(&mut f, &dialog, "web", "shop/web:2.2");
    set_text(&mut f, &dialog, "migrate", "shop/migrate:2");
    let edits = f.vcx.update(|_, cx| dialog.read(cx).edits(cx));
    let described: Vec<_> = edits.iter().map(|e| e.describe()).collect();
    assert_eq!(
        described,
        [
            "migrate: shop/migrate:1 -> shop/migrate:2",
            "web: shop/web:2.1 -> shop/web:2.2"
        ]
    );
    assert!(drawn(&mut f, "setimage-diff-0") && drawn(&mut f, "setimage-diff-1"));
    assert!(!drawn(&mut f, "setimage-no-changes"));
    assert!(f.vcx.update(|_, cx| dialog.read(cx).can_submit(cx)));
    // Putting one back removes it from the diff.
    set_text(&mut f, &dialog, "migrate", "shop/migrate:1");
    assert_eq!(f.vcx.update(|_, cx| dialog.read(cx).edits(cx)).len(), 1);
}

#[gpui::test]
fn an_empty_image_or_one_with_a_space_is_rejected_and_nothing_is_sent(cx: &mut TestAppContext) {
    let mut f = window(cx, [multi_container()]);
    let dialog = press_i(&mut f);
    set_text(&mut f, &dialog, "sidecar", "envoy:2");
    for (text, says) in [
        ("", "Enter an image"),
        ("   ", "Enter an image"),
        ("nginx 1", "spaces"),
    ] {
        set_text(&mut f, &dialog, "web", text);
        let (problem, can) = f.vcx.update(|_, cx| {
            let d = dialog.read(cx);
            (d.problem(1, cx), d.can_submit(cx))
        });
        assert!(problem.expect(text).contains(says), "{text}");
        assert!(
            !can,
            "{text:?}: another container changed, still not allowed"
        );
        assert!(drawn(&mut f, "setimage-problem-1"), "{text:?}");
        submit(&mut f, &dialog);
        assert!(dialog_open(&mut f), "{text:?}: still asking");
    }
    assert!(f.state.audit_log().is_empty(), "nothing reached the bus");
    assert!(f.ports().resources.mutating_calls().is_empty());
}

fn dialog_open(f: &mut Fixture) -> bool {
    dialog(f).is_some()
}

#[gpui::test]
fn confirming_sends_a_strategic_patch_with_only_the_changed_container_and_audits_it(
    cx: &mut TestAppContext,
) {
    let mut f = window(cx, [multi_container()]);
    let dialog = press_i(&mut f);
    set_text(&mut f, &dialog, "web", "shop/web:2.2");
    submit(&mut f, &dialog);

    assert!(!dialog_open(&mut f), "the dialog closed");
    let patches: Vec<_> = f
        .ports()
        .resources
        .mutating_calls()
        .into_iter()
        .map(|call| match call {
            ResourceCall::Patch { patch, options, .. } => (options.dry_run, patch.kind, patch.body),
            other => panic!("expected a patch, got {other:?}"),
        })
        .collect();
    let body = json!({"spec": {"template": {"spec": {
        "containers": [{"name": "web", "image": "shop/web:2.2"}]
    }}}});
    // The guard's dry run before it asks (E19-S07), the handler's, the patch.
    assert_eq!(
        patches,
        [
            (true, PatchKind::Strategic, body.clone()),
            (true, PatchKind::Strategic, body.clone()),
            (false, PatchKind::Strategic, body)
        ]
    );
    assert_eq!(
        audit(&f, "workload::SetImage"),
        [(
            AuditOutcome::Succeeded,
            "containers=web web=shop/web:2.1->shop/web:2.2 rv=10".to_owned()
        )]
    );
    let toasts = crate::actions::tests::toasts(&mut f);
    assert!(
        toasts
            .iter()
            .any(|t| t.message.contains("web: shop/web:2.1 -> shop/web:2.2")),
        "{toasts:?}"
    );
}

#[gpui::test]
fn a_refusal_stays_in_the_dialog_with_the_servers_message(cx: &mut TestAppContext) {
    let mut f = window(cx, [multi_container()]);
    let dialog = press_i(&mut f);
    f.ports()
        .resources
        .script()
        .patch
        .push_err(oxikube_domain::OxiError::forbidden(
            "admission webhook \"policy\" denied the request: image not allowed",
        ));
    set_text(&mut f, &dialog, "web", "evil/web:1");
    submit(&mut f, &dialog);
    let (stage, error) = f.vcx.update(|_, cx| {
        (
            dialog.read(cx).stage(),
            dialog.read(cx).error().map(str::to_owned),
        )
    });
    assert_eq!(stage, SetImageStage::Editing, "back to the form");
    assert!(error.expect("a message").contains("image not allowed"));
    assert!(drawn(&mut f, "setimage-error"));
    // The webhook refused the guard's dry run (E19-S07): nothing was asked, nothing was sent.
    let audit = f.state.audit_log();
    assert!(audit.iter().all(|r| r.dry_run), "{audit:?}");
    assert_eq!(audit[0].outcome, AuditOutcome::Failed);
}

#[gpui::test]
fn cancelling_sends_nothing(cx: &mut TestAppContext) {
    let mut f = window(cx, [multi_container()]);
    let dialog = press_i(&mut f);
    set_text(&mut f, &dialog, "web", "shop/web:9");
    f.vcx
        .update(|_, cx| dialog.update(cx, |d, cx| d.cancel(cx)));
    f.settle();
    assert!(!dialog_open(&mut f));
    assert!(f.state.audit_log().is_empty());
    assert!(f.ports().resources.mutating_calls().is_empty());
}

#[gpui::test]
fn a_daemonset_takes_a_new_image_through_the_same_dialog(cx: &mut TestAppContext) {
    let mut f = window(cx, [daemonset()]);
    let table = f.open(daemonsets());
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.keys(&table, "i");
    f.settle();
    let dialog = dialog(&mut f).expect("open");
    set_text(&mut f, &dialog, "agent", "agent:2");
    submit(&mut f, &dialog);
    assert!(!dialog_open(&mut f));
    assert_eq!(
        audit(&f, "workload::SetImage"),
        [(
            AuditOutcome::Succeeded,
            "containers=agent agent=agent:1->agent:2 rv=4".to_owned()
        )]
    );
}

fn window_detail(cx: &mut TestAppContext) -> Detail {
    Detail::with_workloads(
        cx,
        [
            statefulsets(),
            replicasets(),
            daemonsets(),
            kind("", "Pod", "pods"),
        ],
        [multi_container(), daemonset()],
    )
}

fn detail_dialog(d: &mut Detail) -> Option<Entity<SetImageDialog>> {
    let tabs = d.f.tabs.clone();
    d.f.vcx.update(|_, cx| {
        let tab = tabs.read(cx).tab(&cluster())?.clone();
        let layer = tab.read(cx).workspace().read(cx).modal_layer().clone();
        layer.read(cx).active_modal::<SetImageDialog>()
    })
}

#[gpui::test]
fn r_and_i_in_the_focused_drawer_do_what_the_header_tooltips_say(cx: &mut TestAppContext) {
    let mut d = window_detail(cx);
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    let table = d.f.open(deployments());
    d.f.keys(&table, "j enter");
    d.settle();
    assert!(d.drawer_view().is_some(), "Enter focuses the drawer");

    // `r` sends `workload::Restart` for the shown object.
    d.f.dispatcher.clear();
    d.f.vcx.simulate_keystrokes("r");
    d.settle();
    let sent = d.f.dispatcher.sent();
    assert!(
        matches!(&sent[..], [Command::WorkloadRestart { target }] if target == &web_ref()),
        "{sent:?}"
    );

    // `i` opens the set-image dialog.
    d.f.vcx.simulate_keystrokes("i");
    d.settle();
    assert!(detail_dialog(&mut d).is_some(), "i opens the dialog");
}

#[gpui::test]
fn the_detail_header_has_both_buttons_and_the_palette_runs_them(cx: &mut TestAppContext) {
    use oxikube_workspace::command_surface::CommandSurface as _;
    let mut d = window_detail(cx);
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    let view = d.open(&web_ref());
    assert!(d.shown("detail-restart") && d.shown("detail-set-image"));

    // The set-image button opens the dialog.
    d.click("detail-set-image");
    assert!(
        detail_dialog(&mut d).is_some(),
        "the button opens the dialog"
    );
    d.f.vcx.update(|window, cx| {
        let tabs = d.f.tabs.clone();
        let tab = tabs.read(cx).tab(&cluster()).cloned().unwrap();
        let ws = tab.read(cx).workspace().clone();
        ws.update(cx, |ws, cx| ws.hide_modal(window, cx));
    });

    // The restart button sends `workload::Restart` for this object.
    d.f.dispatcher.clear();
    d.click("detail-restart");
    let sent = d.f.dispatcher.sent();
    assert!(
        matches!(&sent[..], [Command::WorkloadRestart { target }] if target == &web_ref()),
        "{sent:?}"
    );

    // The palette lists and runs them from the focused detail.
    let own = d.f.vcx.update(|_, cx| view.read(cx).own_commands(cx));
    assert!(
        own.contains(&CommandId::WORKLOAD_RESTART) && own.contains(&CommandId::WORKLOAD_SET_IMAGE)
    );
    d.f.dispatcher.clear();
    let ran = d.f.vcx.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.run_command(CommandId::WORKLOAD_RESTART, vec![web_ref()], window, cx)
        })
    });
    assert!(ran);
    assert!(matches!(
        &d.f.dispatcher.sent()[..],
        [Command::WorkloadRestart { .. }]
    ));

    // Read-only: the buttons are disabled with the reason and the palette no longer offers them.
    d.f.sessions.set_read_only(&cluster(), true).unwrap();
    d.settle();
    let state = d.f.vcx.update(|_, cx| view.read(cx).rollout_state());
    assert!(
        state
            .expect("offered")
            .expect("blocked")
            .contains("read-only")
    );
    d.f.dispatcher.clear();
    d.click("detail-restart");
    assert!(
        d.f.dispatcher.sent().is_empty(),
        "a disabled button sends nothing"
    );
    let own = d.f.vcx.update(|_, cx| view.read(cx).own_commands(cx));
    assert!(!own.contains(&CommandId::WORKLOAD_RESTART));
}

#[gpui::test]
fn a_daemonsets_header_has_the_buttons_and_a_replicasets_does_not(cx: &mut TestAppContext) {
    let rs = Resource::from_json(json!({
        "apiVersion": "apps/v1", "kind": "ReplicaSet",
        "metadata": {"name": "web-5d", "namespace": "shop", "uid": "rs-uid", "resourceVersion": "3",
                     "creationTimestamp": "2026-01-01T00:00:00Z"},
        "spec": {"replicas": 1, "selector": {"matchLabels": {"app": "web"}}},
        "status": {"replicas": 1}
    }))
    .unwrap();
    let mut d = Detail::with_workloads(
        cx,
        [statefulsets(), replicasets(), daemonsets()],
        [daemonset(), rs],
    );
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    let ds = ResourceRef::namespaced(
        cluster(),
        Gvk::new("apps", "v1", "DaemonSet"),
        "shop",
        "agent",
    );
    d.open(&ds);
    assert!(d.shown("detail-restart") && d.shown("detail-set-image"));
    let rs_ref = ResourceRef::namespaced(
        cluster(),
        Gvk::new("apps", "v1", "ReplicaSet"),
        "shop",
        "web-5d",
    );
    d.open(&rs_ref);
    assert!(!d.shown("detail-restart") && !d.shown("detail-set-image"));
}
