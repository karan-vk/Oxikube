//! Download and upload through the service: temp-then-rename, cancel, safe unpack, the tar
//! archive an upload sends, and the board.

use std::path::PathBuf;
use std::time::Duration;

use futures::{StreamExt as _, stream};
use oxikube_domain::ErrorKind;
use oxikube_ports::ExitStatus;
use oxikube_testkit::ExecScript;

use super::{Fixture, archive_of};
use crate::files::{DownloadRequest, TransferState};
use crate::testing::pod;

fn request(path: &str, directory: bool, destination: &str) -> DownloadRequest {
    DownloadRequest {
        pod: pod("a", "web-0"),
        container: Some("app".into()),
        path: path.into(),
        directory,
        destination: PathBuf::from(destination),
        as_archive: false,
        total_hint: Some(11),
    }
}

#[tokio::test]
async fn a_file_is_fetched_with_cat_and_written_whole_under_the_chosen_name() {
    let f = Fixture::new(false);
    f.script(ExecScript::new().stdout("hello ").stdout("world"));
    let outcome = f
        .service
        .download(&request("/etc/motd", false, "/dl/motd"))
        .await
        .expect("downloaded");
    assert_eq!(outcome.bytes, 11);
    assert_eq!(f.fs.file("/dl/motd").as_deref(), Some(&b"hello world"[..]));
    let command = &f.commands()[0];
    assert!(command.iter().any(|part| part == "cat"), "{command:?}");

    let board = f.service.board().snapshot();
    assert_eq!(board.len(), 1);
    assert_eq!(board[0].state, TransferState::Done);
    assert_eq!(board[0].bytes, 11);
    assert_eq!(board[0].remote, "/etc/motd");
    assert_eq!(board[0].local, "motd");
    assert!(board[0].place.contains("web-0 (app)"));
}

#[tokio::test]
async fn a_failed_download_leaves_nothing_under_the_final_name_and_says_why() {
    let f = Fixture::new(false);
    f.script(
        ExecScript::new()
            .stdout("partial")
            .stderr("cat: /etc/shadow: Permission denied")
            .exit(Ok(ExitStatus::with_code(1))),
    );
    let error = f
        .service
        .download(&request("/etc/shadow", false, "/dl/shadow"))
        .await
        .expect_err("denied");
    assert_eq!(error.kind(), ErrorKind::Forbidden, "{error}");
    assert_eq!(f.fs.file("/dl/shadow"), None, "no half-written file");
    let board = f.service.board().snapshot();
    assert!(matches!(&board[0].state, TransferState::Failed(m) if m.contains("Permission denied")));
}

#[tokio::test]
async fn a_missing_tar_is_reported_by_name_in_the_transfers_panel() {
    let f = Fixture::new(false);
    f.script(ExecScript::new().exit(Ok(ExitStatus::with_code(127))));
    let error = f
        .service
        .download(&request("/var/log", true, "/dl/log"))
        .await
        .expect_err("no tar");
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    let board = f.service.board().snapshot();
    let TransferState::Failed(message) = &board[0].state else {
        panic!("failed: {:?}", board[0]);
    };
    assert!(
        message.contains("tar") && message.contains("container app"),
        "{message}"
    );
}

#[tokio::test]
async fn cancelling_stops_the_remote_process_and_leaves_no_file() {
    let f = Fixture::new(false);
    let script = ExecScript::new().stdout_stream(
        stream::once(async { Ok(b"first chunk".to_vec()) })
            .chain(stream::pending())
            .boxed(),
    );
    let capture = script.capture();
    f.script(script);
    let service = f.service.clone();
    let task =
        tokio::spawn(async move { service.download(&request("/big", false, "/dl/big")).await });
    // Wait for the transfer to be on the board and to have moved bytes.
    let board = f.service.board().clone();
    let id = loop {
        if let Some(info) = board.snapshot().first()
            && info.bytes > 0
        {
            break info.id;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    assert!(board.cancel(id));
    let error = task.await.expect("joined").expect_err("cancelled");
    assert_eq!(error.kind(), ErrorKind::Conflict);
    assert_eq!(f.fs.file("/dl/big"), None);
    assert_eq!(board.snapshot()[0].state, TransferState::Cancelled);
    assert!(
        capture.aborted(),
        "dropping the session stops the remote process"
    );
    assert!(
        !board.cancel(id),
        "a finished transfer cannot be cancelled again"
    );
}

#[tokio::test]
async fn a_directory_is_unpacked_safely_under_the_destination() {
    let f = Fixture::new(false);
    let archive = archive_of(&[("log/app.log", b"line"), ("log/sub/x", b"y")]);
    f.script(ExecScript::new().stdout(archive));
    let outcome = f
        .service
        .download(&request("/var/log", true, "/dl"))
        .await
        .expect("unpacked");
    assert_eq!(outcome.files, 2);
    assert_eq!(f.fs.file("/dl/log/app.log").as_deref(), Some(&b"line"[..]));
    assert_eq!(f.fs.file("/dl/log/sub/x").as_deref(), Some(&b"y"[..]));
}

#[tokio::test]
async fn a_hostile_archive_is_refused_before_it_writes_outside() {
    let f = Fixture::new(false);
    let mut archive = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(1);
    header.set_mode(0o644);
    header.set_path("ok").expect("path");
    header.set_cksum();
    // A name that climbs out, written raw because the builder refuses `..`.
    let mut raw = header.clone();
    raw.as_old_mut().name[..8].copy_from_slice(b"../evil\0");
    raw.set_cksum();
    archive.append(&raw, &b"x"[..]).expect("append");
    f.script(ExecScript::new().stdout(archive.into_inner().expect("finish")));
    let error = f
        .service
        .download(&request("/x", true, "/dl"))
        .await
        .expect_err("refused");
    assert_eq!(error.kind(), ErrorKind::Validation);
    assert_eq!(f.fs.file("/evil"), None);
    assert_eq!(f.fs.file("/dl/../evil"), None);
}

#[tokio::test]
async fn a_directory_can_be_saved_as_one_tar_file() {
    let f = Fixture::new(false);
    let archive = archive_of(&[("log/app.log", b"line")]);
    f.script(ExecScript::new().stdout(archive.clone()));
    let mut asked = request("/var/log", true, "/dl/log.tar");
    asked.as_archive = true;
    f.service.download(&asked).await.expect("saved");
    assert_eq!(f.fs.file("/dl/log.tar"), Some(archive));
}

#[tokio::test]
async fn an_upload_sends_one_file_as_a_tar_with_its_name_and_closes_stdin() {
    use crate::files::UploadRequest;
    use crate::guard::Mutation;
    use crate::testing::id;

    let f = Fixture::new(false);
    f.fs.insert("/home/me/notes.txt", b"hello".to_vec());
    // The existence check says no; then the upload itself.
    f.script(ExecScript::new().exit(Ok(ExitStatus::with_code(1))));
    let upload = ExecScript::new();
    let capture = upload.capture();
    f.script(upload);
    let mutation = Mutation::new(id("a"), f.h.resources("a"), false);
    let receipt = f
        .service
        .upload(
            &mutation,
            &UploadRequest {
                pod: pod("a", "web-0"),
                container: Some("app".into()),
                dest_dir: "/tmp".into(),
                local: PathBuf::from("/home/me/notes.txt"),
                size: 5,
                overwrite: false,
            },
        )
        .await
        .expect("uploaded");
    assert!(receipt.bytes > 5, "the archive is larger than the file");
    assert!(capture.stdin_closed());
    let sent = capture.stdin();
    let mut archive = tar::Archive::new(&sent[..]);
    let entry = archive
        .entries()
        .expect("entries")
        .next()
        .expect("one")
        .expect("entry");
    assert_eq!(entry.path().expect("path").to_str(), Some("notes.txt"));
    assert_eq!(entry.header().size().expect("size"), 5);
    let commands = f.commands();
    assert_eq!(commands[1][..2], ["tar".to_owned(), "-xmf".to_owned()]);
    assert_eq!(commands[1].last().map(String::as_str), Some("/tmp"));
    let board = f.service.board().snapshot();
    assert_eq!(board[0].state, TransferState::Done);
    assert_eq!(board[0].remote, "/tmp/notes.txt");
}

#[tokio::test]
async fn an_upload_refuses_to_replace_unless_told_and_notices_a_changed_file() {
    use crate::files::UploadRequest;
    use crate::guard::Mutation;
    use crate::testing::id;

    let f = Fixture::new(false);
    f.fs.insert("/home/me/notes.txt", b"hello".to_vec());
    let mutation = Mutation::new(id("a"), f.h.resources("a"), false);
    let mut asked = UploadRequest {
        pod: pod("a", "web-0"),
        container: None,
        dest_dir: "/tmp".into(),
        local: PathBuf::from("/home/me/notes.txt"),
        size: 5,
        overwrite: false,
    };
    // `test -e` succeeds: the file is there.
    f.script(ExecScript::new());
    let error = f
        .service
        .upload(&mutation, &asked)
        .await
        .expect_err("exists");
    assert_eq!(error.kind(), ErrorKind::Conflict);
    assert!(error.message().contains("already exists"));
    assert_eq!(f.commands().len(), 1, "nothing was sent");

    // Told to replace, but the file grew since it was measured.
    asked.overwrite = true;
    asked.size = 3;
    let error = f
        .service
        .upload(&mutation, &asked)
        .await
        .expect_err("changed");
    assert!(error.message().contains("changed"), "{error}");

    // A plan reports the size and the clash.
    f.script(ExecScript::new());
    let plan = f
        .service
        .plan_upload(
            &pod("a", "web-0"),
            None,
            "/tmp",
            &PathBuf::from("/home/me/notes.txt"),
        )
        .await
        .expect("plan");
    assert!(plan.exists);
    assert_eq!((plan.name.as_str(), plan.size), ("notes.txt", 5));
}

#[tokio::test]
async fn a_download_dropped_mid_flight_is_not_left_running_on_the_board() {
    let f = Fixture::new(false);
    let script = ExecScript::new().stdout_stream(
        stream::once(async { Ok(b"first chunk".to_vec()) })
            .chain(stream::pending())
            .boxed(),
    );
    let capture = script.capture();
    f.script(script);
    let service = f.service.clone();
    let task =
        tokio::spawn(async move { service.download(&request("/big", false, "/dl/big")).await });
    let board = f.service.board().clone();
    while board.snapshot().first().is_none_or(|info| info.bytes == 0) {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // The tab closes: its task is aborted, not cancelled through the board.
    task.abort();
    let _ = task.await;
    assert_eq!(board.running(), 0, "nothing is left running");
    assert_eq!(board.snapshot()[0].state, TransferState::Cancelled);
    assert!(capture.aborted());
    assert_eq!(f.fs.file("/dl/big"), None);
}
