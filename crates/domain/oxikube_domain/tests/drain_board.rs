//! The drain's flags and its board: the kubectl line, the defaults, the payload, and the
//! progress events folded into one row per pod.

use std::time::Duration;

use oxikube_domain::Command;
use oxikube_domain::drain::{
    DrainBoard, DrainFlags, DrainPhase, DrainProgress, DrainSummary, PodDrainState, PodRef,
    SkipReason, SkippedPod,
};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};

fn pod(name: &str) -> PodRef {
    PodRef {
        namespace: "shop".into(),
        name: name.into(),
        uid: Some(format!("{name}-uid")),
    }
}

#[test]
fn the_defaults_ignore_daemonsets_and_wait_five_minutes() {
    let flags = DrainFlags::default();
    assert!(flags.ignore_daemonsets && !flags.delete_emptydir_data && !flags.force);
    assert_eq!(flags.grace_period_secs, None);
    assert_eq!(
        flags.kubectl_command("worker-1"),
        "kubectl drain worker-1 --ignore-daemonsets --timeout=300s"
    );
    let options = flags.options();
    assert_eq!(options.timeout, Duration::from_secs(300));
    assert!(options.ignore_daemonsets && options.grace_period_secs.is_none());
    assert_eq!(
        flags.audit_words(),
        "ignore_daemonsets=true delete_emptydir_data=false force=false grace=pod timeout=300s"
    );
}

#[test]
fn every_flag_shows_in_the_kubectl_line() {
    let flags = DrainFlags {
        ignore_daemonsets: false,
        delete_emptydir_data: true,
        force: true,
        grace_period_secs: Some(0),
        timeout_secs: 60,
    };
    assert_eq!(
        flags.kubectl_command("n"),
        "kubectl drain n --delete-emptydir-data --force --grace-period=0 --timeout=60s"
    );
    assert_eq!(flags.options().grace_period_secs, Some(0));
}

#[test]
fn out_of_range_numbers_are_refused() {
    let zero = DrainFlags {
        timeout_secs: 0,
        ..DrainFlags::default()
    };
    assert!(zero.validate().is_err());
    let long_grace = DrainFlags {
        grace_period_secs: Some(100_000),
        ..DrainFlags::default()
    };
    assert!(long_grace.validate().is_err());
    assert!(DrainFlags::default().validate().is_ok());
}

#[test]
fn the_payload_is_flat_and_missing_flags_take_the_defaults() {
    let target = ResourceRef::cluster_scoped(
        ClusterId::new("kubeconfig", &ContextName::new("kind")),
        Gvk::new("", "v1", "Node"),
        "worker-1",
    );
    let json = serde_json::json!({
        "type": "node::Drain",
        "target": serde_json::to_value(&target).unwrap(),
        "force": true,
    });
    let command: Command = serde_json::from_value(json).expect("a drain command");
    let Command::NodeDrain { target, flags } = &command else {
        panic!("not a drain: {command:?}");
    };
    assert_eq!(&*target.name, "worker-1");
    assert!(flags.force && flags.ignore_daemonsets);
    assert_eq!(flags.timeout_secs, 300);
    let back = serde_json::to_value(&command).expect("serialise");
    assert_eq!(back["force"], true);
    assert_eq!(back["ignore_daemonsets"], true);
    assert!(back.get("flags").is_none(), "flat: {back}");
    assert_eq!(serde_json::from_value::<Command>(back).unwrap(), command);
}

#[test]
fn the_board_follows_each_pod_from_pending_to_deleted() {
    let mut board = DrainBoard::new();
    assert_eq!(board.phase(), &DrainPhase::Starting);
    assert!(board.is_running());
    let skipped = vec![SkippedPod {
        pod: pod("agent"),
        reason: SkipReason::DaemonSet,
    }];
    board.apply(&DrainProgress::Planned {
        evict: vec![pod("a"), pod("b")],
        skipped: skipped.clone(),
    });
    assert_eq!(board.phase(), &DrainPhase::Planned);
    assert_eq!(board.skipped(), skipped);
    assert!(
        board
            .rows()
            .iter()
            .all(|row| row.state == PodDrainState::Pending)
    );

    board.apply(&DrainProgress::Cordoned { already: false });
    assert_eq!(board.was_cordoned(), Some(false));
    assert_eq!(board.phase(), &DrainPhase::Evicting);

    board.apply(&DrainProgress::Evicting {
        pod: pod("a"),
        attempt: 1,
    });
    board.apply(&DrainProgress::Blocked {
        pod: pod("a"),
        attempt: 1,
        reason: "budget web".into(),
        retry_in: Duration::from_secs(1),
    });
    assert_eq!(board.rows()[0].state.label(), "Blocked by PDB");
    assert_eq!(board.counts().blocked, 1);
    board.apply(&DrainProgress::Evicting {
        pod: pod("a"),
        attempt: 2,
    });
    board.apply(&DrainProgress::Evicted { pod: pod("a") });
    assert_eq!(board.rows()[0].state, PodDrainState::Terminating);
    board.apply(&DrainProgress::Gone { pod: pod("a") });
    board.apply(&DrainProgress::PodFailed {
        pod: pod("b"),
        error: "still terminating at the timeout".into(),
    });
    // A pod only the second look after the cordon found gets a row at the end.
    board.apply(&DrainProgress::Gone { pod: pod("late") });

    let counts = board.counts();
    assert_eq!((counts.total, counts.deleted, counts.failed), (3, 2, 1));
    assert_eq!(board.rows()[2].pod.name, "late");
    assert!(board.rows().iter().all(|row| matches!(
        row.state,
        PodDrainState::Deleted | PodDrainState::Failed { .. }
    )));

    let summary = DrainSummary {
        node: "n".into(),
        evicted: vec![pod("a"), pod("late")],
        skipped,
        failed: vec![pod("b")],
        dry_run: false,
    };
    board.apply(&DrainProgress::Finished(summary.clone()));
    assert_eq!(board.phase(), &DrainPhase::Finished(summary));
    assert!(!board.is_running());
    // A finished drain is not cancelled after the fact.
    board.cancel();
    assert!(matches!(board.phase(), DrainPhase::Finished(_)));
}

#[test]
fn a_failed_or_cancelled_drain_keeps_its_rows() {
    let mut board = DrainBoard::new();
    board.apply(&DrainProgress::Planned {
        evict: vec![pod("a")],
        skipped: Vec::new(),
    });
    board.apply(&DrainProgress::Evicting {
        pod: pod("a"),
        attempt: 1,
    });
    let mut failed = board.clone();
    failed.fail("forbidden");
    assert_eq!(failed.phase(), &DrainPhase::Failed("forbidden".into()));
    board.cancel();
    assert_eq!(board.phase(), &DrainPhase::Cancelled);
    assert_eq!(
        board.rows()[0].state,
        PodDrainState::Evicting { attempt: 1 }
    );
}

#[test]
fn a_planned_pod_gone_before_the_second_look_leaves_no_pending_row() {
    let mut board = DrainBoard::new();
    board.apply(&DrainProgress::Planned {
        evict: vec![pod("a"), pod("b"), pod("c")],
        skipped: Vec::new(),
    });
    board.apply(&DrainProgress::Cordoned { already: false });
    // `b` completed between the first list and the re-list after the cordon: no event names it.
    for name in ["a", "c"] {
        board.apply(&DrainProgress::Evicting {
            pod: pod(name),
            attempt: 1,
        });
        board.apply(&DrainProgress::Evicted { pod: pod(name) });
        board.apply(&DrainProgress::Gone { pod: pod(name) });
    }
    board.apply(&DrainProgress::Finished(DrainSummary {
        node: "n".into(),
        evicted: vec![pod("a"), pod("c")],
        skipped: Vec::new(),
        failed: Vec::new(),
        dry_run: false,
    }));

    let names: Vec<_> = board
        .rows()
        .iter()
        .map(|row| row.pod.name.as_str())
        .collect();
    assert_eq!(names, ["a", "c"]);
    let counts = board.counts();
    assert_eq!((counts.total, counts.deleted, counts.failed), (2, 2, 0));
    // The index still points at the right rows after the drop.
    board.apply(&DrainProgress::PodFailed {
        pod: pod("c"),
        error: "late".into(),
    });
    assert_eq!(board.rows()[1].state.label(), "Failed");
    assert_eq!(board.rows().len(), 2);
}

#[test]
fn a_dry_run_keeps_its_planned_rows_pending() {
    let mut board = DrainBoard::new();
    board.apply(&DrainProgress::Planned {
        evict: vec![pod("a"), pod("b")],
        skipped: Vec::new(),
    });
    board.apply(&DrainProgress::Finished(DrainSummary {
        node: "n".into(),
        evicted: vec![pod("a"), pod("b")],
        skipped: Vec::new(),
        failed: Vec::new(),
        dry_run: true,
    }));
    assert_eq!(board.counts().total, 2);
    assert!(
        board
            .rows()
            .iter()
            .all(|row| row.state == PodDrainState::Pending)
    );
}
