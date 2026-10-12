//! The drain dialog (E12-S10): `r`, the menu and the palette open it; it shows the plan for the
//! chosen flags before anything changes, wants the node's name typed, runs `node::Drain` through
//! the real bus and guard over a scripted drain port, renders per-pod progress, and cancels.

use std::time::Duration;

use gpui::{Entity, TestAppContext};
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::drain::{
    DrainPhase, DrainProgress, DrainSummary, PodDrainState, PodRef, SkipReason, SkippedPod,
};
use oxikube_ports::{ClusterPrefs, ClusterPrefsTable, ListPage};
use oxikube_testkit::{DrainRun, ResourceCall, node, pod};

use super::super::drain::{DrainDialog, DrainLoaded, DrainStage, DrainSwitch, RowTone};
use crate::actions::tests::nodes_kind;
use crate::table::ResourceTable;
use crate::table::tests::fixture::{Fixture, cluster};

fn pod_ref(namespace: &str, name: &str) -> PodRef {
    PodRef {
        namespace: namespace.into(),
        name: name.into(),
        uid: None,
    }
}

/// A window with the Nodes table open on `worker-1`, the cursor on it.
fn nodes_table(cx: &mut TestAppContext) -> (Fixture, Entity<ResourceTable>) {
    let mut f = Fixture::with_exec(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    let ports = f.ports();
    ports.discovery.set_kinds([nodes_kind()]);
    ports.resources.insert(node().name("worker-1").build());
    f.connect_with([]);
    let table = f.open(nodes_kind());
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    (f, table)
}

/// What the node's pod list answers (the fake does not filter by field): a ReplicaSet pod, a
/// DaemonSet pod and a pod no controller manages.
fn script_pods(f: &Fixture) {
    let on = |b: oxikube_testkit::PodBuilder| b.node("worker-1").running().build();
    f.ports()
        .resources
        .script()
        .list
        .push_ok(ListPage::complete(vec![
            on(pod()
                .namespace("shop")
                .name("web-1")
                .controlled_by("ReplicaSet", "web")),
            on(pod()
                .namespace("kube-system")
                .name("agent")
                .controlled_by("DaemonSet", "agent")),
            on(pod().namespace("shop").name("lone")),
        ]));
}

fn dialog(f: &mut Fixture) -> Option<Entity<DrainDialog>> {
    let tabs = f.tabs.clone();
    f.vcx.update(|_, cx| {
        let tab = tabs.read(cx).tab(&cluster())?.clone();
        let layer = tab.read(cx).workspace().read(cx).modal_layer().clone();
        layer.read(cx).active_modal::<DrainDialog>()
    })
}

/// Presses `r` on the cursor row and returns the dialog, its pods read.
fn press_r(f: &mut Fixture, table: &Entity<ResourceTable>) -> Entity<DrainDialog> {
    script_pods(f);
    f.keys(table, "r");
    f.settle();
    dialog(f).expect("r opens the drain dialog")
}

fn with<R>(
    f: &mut Fixture,
    dialog: &Entity<DrainDialog>,
    run: impl FnOnce(&mut DrainDialog, &mut gpui::Window, &mut gpui::Context<DrainDialog>) -> R,
) -> R {
    let r = f
        .vcx
        .update(|window, cx| dialog.update(cx, |d, cx| run(d, window, cx)));
    f.settle();
    r
}

fn draw(f: &mut Fixture) {
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
}

/// The drain of `web-1` and `lone` (forced): `web-1` is held by a budget once, then both go.
fn forced_run() -> DrainRun {
    let skipped = vec![SkippedPod {
        pod: pod_ref("kube-system", "agent"),
        reason: SkipReason::DaemonSet,
    }];
    let (web, lone) = (pod_ref("shop", "web-1"), pod_ref("shop", "lone"));
    DrainRun::of([
        DrainProgress::Planned {
            evict: vec![web.clone(), lone.clone()],
            skipped: skipped.clone(),
        },
        DrainProgress::Cordoned { already: false },
        DrainProgress::Evicting {
            pod: web.clone(),
            attempt: 1,
        },
        DrainProgress::Blocked {
            pod: web.clone(),
            attempt: 1,
            reason: "The disruption budget web needs 2 healthy pods.".into(),
            retry_in: Duration::from_secs(1),
        },
        DrainProgress::Evicting {
            pod: web.clone(),
            attempt: 2,
        },
        DrainProgress::Evicted { pod: web.clone() },
        DrainProgress::Gone { pod: web.clone() },
        DrainProgress::Evicting {
            pod: lone.clone(),
            attempt: 1,
        },
        DrainProgress::Evicted { pod: lone.clone() },
        DrainProgress::Gone { pod: lone.clone() },
        DrainProgress::Finished(DrainSummary {
            node: "worker-1".into(),
            evicted: vec![web, lone],
            skipped,
            failed: Vec::new(),
            dry_run: false,
        }),
    ])
}

#[gpui::test]
fn r_opens_the_dialog_with_the_plan_before_anything_changes(cx: &mut TestAppContext) {
    let (mut f, table) = nodes_table(cx);
    let dialog = press_r(&mut f, &table);
    draw(&mut f);
    for id in [
        "drain-dialog",
        "drain-what",
        "drain-kubectl",
        "drain-rows",
        "drain-blocked",
        "drain-type-input",
        "drain-start",
    ] {
        assert!(f.vcx.debug_bounds(id).is_some(), "{id} is drawn");
    }
    with(&mut f, &dialog, |d, _, cx| {
        assert_eq!(&*d.target().name, "worker-1");
        assert!(matches!(d.loaded_state(), DrainLoaded::Ready(_)));
        assert_eq!(d.stage(), DrainStage::Asking);
        assert_eq!(
            d.kubectl_line(),
            "kubectl drain worker-1 --ignore-daemonsets --timeout=300s"
        );
        let plan = d.plan().expect("planned");
        assert_eq!(plan.evict.len(), 1, "web-1");
        assert_eq!(plan.skipped.len(), 1, "the DaemonSet pod");
        assert_eq!(plan.blocked.len(), 1, "the unmanaged pod");
        // The pod that blocks is listed first, in red, with the flag that allows it.
        let first = &d.lines()[0];
        assert_eq!(&*first.pod, "shop/lone");
        assert_eq!(first.tone, RowTone::Error);
        assert!(first.detail.contains("--force"), "{}", first.detail);
        assert!(!d.can_start(cx), "a pod blocks the drain");
    });
    assert!(f.drain.recorded_calls().is_empty(), "nothing was sent");
    assert!(f.ports().resources.mutating_calls().is_empty());
}

#[gpui::test]
fn the_drain_needs_the_flags_that_unblock_it_and_the_node_name_typed(cx: &mut TestAppContext) {
    let (mut f, table) = nodes_table(cx);
    let dialog = press_r(&mut f, &table);
    with(&mut f, &dialog, |d, _, cx| {
        d.set_flag(DrainSwitch::Force, true, cx);
        assert_eq!(
            d.kubectl_line(),
            "kubectl drain worker-1 --ignore-daemonsets --force --timeout=300s"
        );
        let plan = d.plan().unwrap();
        assert!(plan.blocked.is_empty());
        assert_eq!(plan.evict.len(), 2);
        assert!(
            d.start_blocker(cx).unwrap().contains("Type worker-1"),
            "the name is still to type"
        );
    });
    with(&mut f, &dialog, |d, window, cx| {
        d.type_name("worker-2", window, cx)
    });
    with(&mut f, &dialog, |d, _, cx| {
        assert!(!d.can_start(cx), "a wrong name");
        d.start(cx);
        assert_eq!(d.stage(), DrainStage::Asking, "nothing starts");
    });
    with(&mut f, &dialog, |d, window, cx| {
        d.type_name("worker-1", window, cx)
    });
    with(&mut f, &dialog, |d, _, cx| assert!(d.can_start(cx)));
    // Turning DaemonSets back on as blockers stops it again.
    with(&mut f, &dialog, |d, _, cx| {
        d.set_flag(DrainSwitch::IgnoreDaemonSets, false, cx);
        assert!(!d.can_start(cx));
        assert!(d.start_blocker(cx).unwrap().contains("DaemonSet"));
    });
    assert!(f.drain.recorded_calls().is_empty());
}

#[gpui::test]
fn the_drain_runs_through_the_guard_and_shows_each_pods_progress(cx: &mut TestAppContext) {
    let (mut f, table) = nodes_table(cx);
    f.drain.push(forced_run());
    for _ in 0..2 {
        f.ports().resources.script().evict.push_ok(());
    }
    let dialog = press_r(&mut f, &table);
    with(&mut f, &dialog, |d, _, cx| {
        d.set_flag(DrainSwitch::Force, true, cx)
    });
    with(&mut f, &dialog, |d, window, cx| {
        d.type_name("worker-1", window, cx)
    });
    with(&mut f, &dialog, |d, _, cx| d.start(cx));
    f.vcx.run_until_parked();
    draw(&mut f);
    assert!(f.vcx.debug_bounds("drain-rows").is_some());
    assert!(f.vcx.debug_bounds("drain-close").is_some());
    with(&mut f, &dialog, |d, _, _| {
        assert_eq!(d.stage(), DrainStage::Done);
        assert!(matches!(d.board().phase(), DrainPhase::Finished(_)));
        let states: Vec<_> = d
            .board()
            .rows()
            .iter()
            .map(|row| (row.pod.name.as_str(), row.state.clone()))
            .collect();
        assert_eq!(
            states,
            [
                ("web-1", PodDrainState::Deleted),
                ("lone", PodDrainState::Deleted)
            ]
        );
        assert_eq!(d.lines().len(), 3, "two pods and the skipped one");
        assert!(d.message_text().unwrap().contains("2 pod(s) evicted"));
        assert!(!d.offers_uncordon(), "every pod went");
    });
    // The algorithm got the flags; the writes went through the guard; the audit has the drain
    // and each pod.
    let calls = f.drain.recorded_calls();
    let [oxikube_testkit::DrainCall::Drain { node, options }] = &calls[..] else {
        panic!("one drain: {calls:?}");
    };
    assert_eq!(node, "worker-1");
    assert!(options.force && options.ignore_daemonsets);
    let evictions = f
        .ports()
        .resources
        .mutating_calls()
        .into_iter()
        .filter(|c| matches!(c, ResourceCall::Evict { .. }))
        .count();
    assert_eq!(evictions, 2);
    let audit = f.state.audit_log();
    let drained: Vec<_> = audit
        .iter()
        .filter(|r| &*r.cmd == "node::Drain")
        .map(|r| (r.target.name.to_string(), r.outcome))
        .collect();
    assert_eq!(
        drained,
        [
            ("web-1".to_owned(), AuditOutcome::Succeeded),
            ("lone".to_owned(), AuditOutcome::Succeeded),
            ("worker-1".to_owned(), AuditOutcome::Succeeded),
        ]
    );
}

#[gpui::test]
fn a_blocked_pod_shows_as_blocked_by_pdb_while_the_drain_runs(cx: &mut TestAppContext) {
    let (mut f, table) = nodes_table(cx);
    let web = pod_ref("shop", "web-1");
    f.drain.push(
        DrainRun::of([
            DrainProgress::Planned {
                evict: vec![web.clone()],
                skipped: Vec::new(),
            },
            DrainProgress::Cordoned { already: false },
            DrainProgress::Evicting {
                pod: web.clone(),
                attempt: 1,
            },
            DrainProgress::Blocked {
                pod: web,
                attempt: 1,
                reason: "The disruption budget web needs 2 healthy pods.".into(),
                retry_in: Duration::from_secs(2),
            },
        ])
        .hold(),
    );
    let dialog = press_r(&mut f, &table);
    with(&mut f, &dialog, |d, _, cx| {
        d.set_flag(DrainSwitch::Force, true, cx)
    });
    with(&mut f, &dialog, |d, window, cx| {
        d.type_name("worker-1", window, cx)
    });
    with(&mut f, &dialog, |d, _, cx| d.start(cx));
    f.vcx.run_until_parked();
    draw(&mut f);
    assert!(f.vcx.debug_bounds("drain-stop").is_some(), "Cancel drain");
    with(&mut f, &dialog, |d, _, _| {
        assert_eq!(d.stage(), DrainStage::Draining);
        assert_eq!(d.board().counts().blocked, 1);
        let line = &d.lines()[0];
        assert_eq!(&*line.status, "Blocked by PDB");
        assert_eq!(line.tone, RowTone::Warning);
        assert!(line.detail.contains("Retrying in 2s"), "{}", line.detail);
    });

    // Cancel: the dispatch is dropped (the guard audits it as cancelled), the node stays
    // cordoned and the dialog offers to uncordon it.
    with(&mut f, &dialog, |d, _, cx| d.cancel_drain(cx));
    f.vcx.run_until_parked();
    with(&mut f, &dialog, |d, _, _| {
        assert_eq!(d.stage(), DrainStage::Done);
        assert_eq!(d.board().phase(), &DrainPhase::Cancelled);
        assert!(d.offers_uncordon());
    });
    draw(&mut f);
    assert!(f.vcx.debug_bounds("drain-uncordon").is_some());
    // The dispatch is gone: the drain no longer runs (the guard queued its cancelled record; the
    // app's tests read it back, `oxikube_app::nodes`).
    with(&mut f, &dialog, |d, _, _| {
        assert!(!d.drain_running(), "the drain stopped");
    });
    assert_eq!(
        f.ports()
            .resources
            .mutating_calls()
            .iter()
            .filter(|c| matches!(c, ResourceCall::Evict { .. }))
            .count(),
        0,
        "nothing was evicted"
    );
    f.dispatcher.clear();
    with(&mut f, &dialog, |d, _, cx| d.uncordon(cx));
    assert!(
        f.dispatcher.sent().contains(&Command::NodeUncordon {
            target: super::node_ref("worker-1")
        }),
        "{:?}",
        f.dispatcher.sent()
    );
}

#[gpui::test]
fn the_menu_offers_drain_on_nodes_and_read_only_greys_it_out(cx: &mut TestAppContext) {
    let (mut f, table) = nodes_table(cx);
    let entries = f.vcx.update(|_, cx| table.read(cx).action_entries(cx));
    let drain = entries
        .iter()
        .find(|e| e.command() == CommandId::NODE_DRAIN)
        .expect("Drain… on a node");
    assert_eq!(drain.label, "Drain…");
    assert!(drain.is_enabled());

    f.sessions.set_prefs_table(
        ClusterPrefsTable::new(ClusterPrefs::default()).with_cluster(
            cluster(),
            ClusterPrefs {
                read_only: true,
                ..ClusterPrefs::default()
            },
        ),
    );
    f.settle();
    let entries = f.vcx.update(|_, cx| table.read(cx).action_entries(cx));
    let drain = entries
        .iter()
        .find(|e| e.command() == CommandId::NODE_DRAIN)
        .unwrap();
    assert!(!drain.is_enabled());
    assert!(drain.reason().unwrap().contains("read-only"));
    f.keys(&table, "r");
    f.settle();
    assert!(
        dialog(&mut f).is_none(),
        "r opens nothing on a read-only cluster"
    );
}
