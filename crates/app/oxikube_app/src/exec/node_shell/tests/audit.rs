//! The second audit record: the end of a node shell, which is the deletion of its pod.

use futures::StreamExt as _;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_ports::ExitStatus;
use oxikube_testkit::FakeTerminalBackend;

use super::{Fixture, custom_prefs};

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    // The service's sweep has a timeout, which needs a runtime with a clock.
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("a runtime")
        .block_on(future)
}

fn flush(f: &Fixture) {
    block_on(f.h.bus.guard().audit().flush()).expect("flush");
}

#[test]
fn closing_the_tab_audits_the_deletion_with_the_same_who_node_image_and_namespace() {
    let f = Fixture::with(custom_prefs());
    f.run().expect("runs");
    let backend = block_on(f.service.open_node_shell(&f.node())).expect("opens");
    assert_eq!(f.h.audit().len(), 1, "only the create so far");

    drop(backend);
    flush(&f);
    let audit = f.h.audit();
    assert_eq!(audit.len(), 2, "{audit:?}");
    let (create, delete) = (&audit[0], &audit[1]);
    assert_eq!(
        create.detail.as_deref(),
        Some("phase=create image=registry.local/tools:2 namespace=ops-debug")
    );
    assert_eq!(
        delete.detail.as_deref(),
        Some("phase=delete image=registry.local/tools:2 namespace=ops-debug")
    );
    for record in [create, delete] {
        assert_eq!(&*record.cmd, "node::Shell");
        assert_eq!(&*record.target.name, "worker-1");
        assert_eq!(&*record.who, "alice");
        assert_eq!(record.initiator, Initiator::Ui);
        assert_eq!(record.outcome, AuditOutcome::Succeeded);
    }
}

#[test]
fn the_shell_exiting_audits_the_deletion_once() {
    let f = Fixture::new();
    f.run().expect("runs");
    let scripted = FakeTerminalBackend::silent();
    scripted.exit(ExitStatus::success());
    f.exec.script().node_shell.push_ok(scripted);
    let backend = block_on(f.service.open_node_shell(&f.node())).expect("opens");
    let events: Vec<_> = block_on(backend.output_stream().collect());
    assert!(!events.is_empty());
    // The record was written when the exit came through, before the stream ended.
    assert_eq!(f.h.audit().len(), 2, "{:?}", f.h.audit());
    assert_eq!(f.h.bus.guard().audit().backlog_len(), 0);
    drop(backend);
    flush(&f);
    assert_eq!(f.h.audit().len(), 2, "dropping afterwards adds nothing");
}

#[test]
fn killing_the_shell_audits_the_deletion() {
    let f = Fixture::new();
    f.run().expect("runs");
    let backend = block_on(f.service.open_node_shell(&f.node())).expect("opens");
    block_on(backend.kill()).expect("kill");
    assert_eq!(f.h.audit().len(), 2);
    drop(backend);
    flush(&f);
    assert_eq!(f.h.audit().len(), 2, "once");
}

#[test]
fn a_shell_that_never_opened_has_no_deletion_to_audit() {
    let f = Fixture::new();
    f.run().expect("runs");
    f.exec
        .script()
        .node_shell
        .push_err(oxikube_domain::OxiError::conflict("ImagePullBackOff"));
    assert!(block_on(f.service.open_node_shell(&f.node())).is_err());
    flush(&f);
    assert_eq!(
        f.h.audit().len(),
        1,
        "the adapter already deleted the failed pod"
    );
}

#[test]
fn a_swept_leftover_is_audited_with_the_user_whose_shell_is_opening() {
    let f = Fixture::with(custom_prefs());
    f.exec
        .script()
        .sweep_node_shells
        .push_ok(vec!["leftover-1".to_owned(), "leftover-2".to_owned()]);
    f.run().expect("runs");
    let _backend = block_on(f.service.open_node_shell(&f.node())).expect("opens");
    // Written by the sweep itself: no manual flush.
    let audit = f.h.audit();
    assert_eq!(audit.len(), 3, "the create and the two sweeps: {audit:?}");
    for (record, pod) in audit[1..].iter().zip(["leftover-1", "leftover-2"]) {
        assert_eq!(&*record.cmd, "node::Shell");
        assert_eq!(&*record.target.name, pod);
        assert_eq!(record.target.namespace(), Some("ops-debug"));
        assert_eq!(&*record.target.gvk.kind, "Pod");
        assert_eq!(
            record.detail.as_deref(),
            Some("phase=sweep namespace=ops-debug")
        );
        assert_eq!(&*record.who, "alice");
        assert_eq!(record.initiator, Initiator::Ui);
        assert_eq!(record.outcome, AuditOutcome::Succeeded);
    }
}

#[test]
fn a_sweep_that_deleted_nothing_writes_nothing() {
    let f = Fixture::new();
    f.run().expect("runs");
    let _backend = block_on(f.service.open_node_shell(&f.node())).expect("opens");
    flush(&f);
    assert_eq!(f.h.audit().len(), 1, "only the create");
}

#[test]
fn quitting_after_closing_the_tab_writes_the_deletion_record_without_another_command() {
    let f = Fixture::new();
    f.run().expect("runs");
    let backend = block_on(f.service.open_node_shell(&f.node())).expect("opens");
    drop(backend);
    // The tab's drop only queued the record; nothing else flushes before the app quits.
    assert_eq!(f.h.audit().len(), 1, "queued, not stored");
    assert_eq!(f.h.bus.guard().audit().backlog_len(), 1);

    block_on(f.service.close_node_shells());
    assert_eq!(f.h.audit().len(), 2, "{:?}", f.h.audit());
    assert_eq!(f.h.bus.guard().audit().backlog_len(), 0);
}

#[test]
fn quitting_with_a_shell_open_deletes_its_pod_and_audits_the_end_once() {
    let f = Fixture::with(custom_prefs());
    f.exec.script().release_node_shells.push_ok(1);
    f.run().expect("runs");
    let backend = block_on(f.service.open_node_shell(&f.node())).expect("opens");

    assert_eq!(block_on(f.service.close_node_shells()), 1);
    assert!(
        f.exec
            .recorded_calls()
            .contains(&oxikube_testkit::ExecPortCall::ReleaseNodeShells),
        "the adapter was told to delete the pods it still has open"
    );
    let audit = f.h.audit();
    assert_eq!(audit.len(), 2, "{audit:?}");
    assert_eq!(
        audit[1].detail.as_deref(),
        Some("phase=delete image=registry.local/tools:2 namespace=ops-debug")
    );
    assert_eq!(&*audit[1].who, "alice");

    // The process is going away, but a second quit signal or a late drop adds nothing.
    block_on(f.service.close_node_shells());
    drop(backend);
    flush(&f);
    assert_eq!(f.h.audit().len(), 2, "once");
}

#[test]
fn quitting_with_no_shell_open_touches_nothing() {
    let f = Fixture::new();
    assert_eq!(block_on(f.service.close_node_shells()), 0);
    assert!(f.exec.recorded_calls().is_empty());
    assert!(f.h.audit().is_empty());
}

#[test]
fn a_failed_release_still_audits_the_shells_and_does_not_panic() {
    let f = Fixture::new();
    f.exec
        .script()
        .release_node_shells
        .push_err(oxikube_domain::OxiError::network("down"));
    f.run().expect("runs");
    let _backend = block_on(f.service.open_node_shell(&f.node())).expect("opens");
    assert_eq!(block_on(f.service.close_node_shells()), 0);
    assert_eq!(
        f.h.audit().len(),
        2,
        "the end of the shell is still recorded"
    );
}
