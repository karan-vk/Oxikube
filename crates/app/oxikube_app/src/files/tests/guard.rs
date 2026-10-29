//! The file commands through the guard: upload blocked read-only, risk by overwrite, the audit
//! record without content, downloads as plain reads, and the runner end to end.

use std::path::PathBuf;

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::ExitStatus;
use oxikube_testkit::ExecScript;

use super::Fixture;
use crate::command_bus::{DispatchError, Outcome};
use crate::files::FileRunner;
use crate::guard::Confirmation;
use crate::testing::{ctx, pod};

fn upload(overwrite: bool) -> Command {
    upload_of(overwrite, 5)
}

fn upload_of(overwrite: bool, size: u64) -> Command {
    Command::PodUploadFile {
        target: pod("a", "web-0"),
        container: Some("app".into()),
        dest_dir: "/tmp".into(),
        local_path: "/home/me/secret-plans/notes.txt".into(),
        size,
        overwrite,
    }
}

fn download() -> Command {
    Command::PodDownloadFile {
        target: pod("a", "web-0"),
        container: Some("app".into()),
        path: "/etc/motd".into(),
        directory: false,
        destination: "/dl/motd".into(),
        as_archive: false,
        size: None,
    }
}

#[tokio::test]
async fn an_upload_is_blocked_on_a_read_only_cluster_for_every_initiator_and_audited() {
    let f = Fixture::new(true);
    for initiator in Initiator::ALL {
        let error = f
            .bus()
            .dispatch(upload(false), ctx(initiator))
            .await
            .unwrap_err();
        assert!(
            matches!(error, DispatchError::ReadOnly { .. }),
            "{initiator}: {error:?}"
        );
        assert_eq!(OxiError::from(error).kind(), ErrorKind::Forbidden);
    }
    assert!(f.commands().is_empty(), "nothing reached the container");
    let audit = f.h.audit();
    assert_eq!(audit.len(), Initiator::ALL.len());
    assert!(audit.iter().all(|r| r.outcome == AuditOutcome::Denied));
}

#[tokio::test]
async fn a_new_file_is_low_risk_and_a_replacement_is_medium_both_with_a_simple_confirmation() {
    let f = Fixture::new(false);
    let Outcome::NeedsConfirmation(new) = f
        .bus()
        .dispatch(upload(false), ctx(Initiator::Ui))
        .await
        .unwrap()
    else {
        panic!("a confirmation");
    };
    assert_eq!((new.tier, new.risk), (ConfirmTier::Simple, Some(Risk::Low)));
    for part in [
        "/home/me/secret-plans/notes.txt",
        "5 bytes",
        "/tmp",
        "container app",
        "web-0",
    ] {
        assert!(new.summary.contains(part), "{part}: {}", new.summary);
    }
    assert!(!new.summary.contains("replaces"));
    f.bus().decline(new.token).await.unwrap();

    let Outcome::NeedsConfirmation(replace) = f
        .bus()
        .dispatch(upload(true), ctx(Initiator::Ui))
        .await
        .unwrap()
    else {
        panic!("a confirmation");
    };
    assert_eq!(
        (replace.tier, replace.risk),
        (ConfirmTier::Simple, Some(Risk::Medium))
    );
    assert!(replace.summary.contains("replaces"), "{}", replace.summary);
    assert!(f.commands().is_empty(), "nothing ran before the answer");
}

#[tokio::test]
async fn the_audit_record_names_container_path_and_size_never_the_content_or_local_dirs() {
    let f = Fixture::new(false);
    f.fs.insert("/home/me/secret-plans/notes.txt", b"s3cr3t".to_vec());
    // `test -e` says no (1), then the upload.
    f.script(ExecScript::new().exit(Ok(ExitStatus::with_code(1))));
    f.script(ExecScript::new());
    let Outcome::NeedsConfirmation(asked) = f
        .bus()
        .dispatch(upload_of(false, 6), ctx(Initiator::Ui))
        .await
        .unwrap()
    else {
        panic!("a confirmation");
    };
    let command = upload_of(false, 6);
    let done = f
        .bus()
        .dispatch(
            command,
            ctx(Initiator::Ui).with_confirmation(Confirmation::simple(asked.token)),
        )
        .await
        .unwrap();
    assert!(matches!(done, Outcome::Completed(_)));
    let audit = f.h.audit();
    let [record] = audit.as_slice() else {
        panic!("one record, got {audit:?}");
    };
    assert_eq!(&*record.cmd, "pod::UploadFile");
    assert_eq!(record.outcome, AuditOutcome::Succeeded);
    assert_eq!(record.target, pod("a", "web-0"));
    let detail = record.detail.as_deref().unwrap_or("");
    assert!(
        detail.starts_with("container=app path=/tmp/notes.txt size=6 overwrite=false"),
        "{detail}"
    );
    let all = serde_json::to_string(record).unwrap();
    assert!(
        !all.contains("s3cr3t") && !all.contains("secret-plans"),
        "{all}"
    );
}

#[tokio::test]
async fn a_download_is_a_plain_read_on_a_read_only_cluster_with_no_confirmation() {
    let f = Fixture::new(true);
    f.script(ExecScript::new().stdout("welcome"));
    let done = f
        .bus()
        .dispatch(download(), ctx(Initiator::Ui))
        .await
        .unwrap();
    let Outcome::Completed(output) = done else {
        panic!("completed without asking");
    };
    assert!(output.message.unwrap().contains("/etc/motd"));
    assert_eq!(f.fs.file("/dl/motd").as_deref(), Some(&b"welcome"[..]));
    assert!(f.h.audit().is_empty(), "a read is not audited");
}

#[tokio::test]
async fn only_the_apps_own_ui_may_start_a_download_because_it_writes_the_local_disk() {
    let f = Fixture::new(false);
    for initiator in [Initiator::Agent, Initiator::Plugin] {
        let error = f
            .bus()
            .dispatch(download(), ctx(initiator))
            .await
            .unwrap_err();
        assert_eq!(
            OxiError::from(error).kind(),
            ErrorKind::Forbidden,
            "{initiator}"
        );
    }
    assert!(f.commands().is_empty(), "nothing reached the container");
    assert_eq!(f.fs.file("/dl/motd"), None, "nothing was written");
    for initiator in [Initiator::Ui, Initiator::Command] {
        f.script(ExecScript::new().stdout("x"));
        f.bus()
            .dispatch(download(), ctx(initiator))
            .await
            .unwrap_or_else(|error| panic!("{initiator}: {error:?}"));
    }
}

#[tokio::test]
async fn the_runner_checks_the_destination_asks_the_guard_and_uploads() {
    let f = Fixture::new(false);
    f.fs.insert("/home/me/a.txt", b"hello".to_vec());
    let runner = FileRunner::new(f.bus(), "alice", f.service.clone());
    // The file exists in the container.
    f.script(ExecScript::new());
    let plan = runner
        .plan_upload(
            &pod("a", "web-0"),
            Some("app"),
            "/tmp",
            &PathBuf::from("/home/me/a.txt"),
        )
        .await
        .unwrap();
    assert!(plan.exists);

    // Replacing: the handler's own check passes because overwrite is on; then the upload.
    let script = ExecScript::new();
    let capture = script.capture();
    f.script(script);
    let message = runner
        .upload(&pod("a", "web-0"), Some("app"), "/tmp", &plan, true)
        .await
        .unwrap();
    assert!(message.contains("a.txt"), "{message}");
    assert!(!capture.stdin().is_empty());
    let audit = f.h.audit();
    assert_eq!(audit.len(), 1);
    assert!(
        audit[0]
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("overwrite=true")
    );
    assert_eq!(f.h.bus.guard().pending_confirmations(), 0);
}

#[tokio::test]
async fn a_dry_run_upload_sends_nothing() {
    let f = Fixture::new(false);
    f.fs.insert("/home/me/secret-plans/notes.txt", b"hello".to_vec());
    f.script(ExecScript::new().exit(Ok(ExitStatus::with_code(1))));
    let Outcome::NeedsConfirmation(asked) = f
        .bus()
        .dispatch(upload(false), ctx(Initiator::Ui).with_dry_run(true))
        .await
        .unwrap()
    else {
        panic!("a confirmation");
    };
    let done = f
        .bus()
        .dispatch(
            upload(false),
            ctx(Initiator::Ui)
                .with_dry_run(true)
                .with_confirmation(Confirmation::simple(asked.token)),
        )
        .await
        .unwrap();
    let Outcome::Completed(output) = done else {
        panic!("completed");
    };
    assert!(output.message.unwrap().starts_with("Dry run"));
    assert_eq!(f.commands().len(), 1, "only the existence check ran");
    assert_eq!(CommandId::POD_UPLOAD_FILE.as_str(), "pod::UploadFile");
}
