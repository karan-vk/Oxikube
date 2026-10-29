//! `TransferService` over a real session manager and the `FakeExecStreamPort`.

use std::sync::Arc;

use futures::StreamExt as _;
use oxikube_domain::ErrorKind;
use oxikube_ports::{ExitStatus, RemotePath, TransferOptions};
use oxikube_testkit::{ExecScript, ExecStreamCall, FakeExecStreamPort};

use super::{chunked, collect};
use crate::guard::Mutation;
use crate::testing::{Harness, id};
use crate::transfer::{DownloadFormat, RemoteEntry, TransferService};

struct Fixture {
    h: Harness,
    service: TransferService,
    exec: Arc<FakeExecStreamPort>,
}

impl Fixture {
    fn new() -> Self {
        let h = Harness::new();
        h.connect("a", false);
        let exec = h.connector.ports_for(&id("a")).exec_stream;
        Self {
            service: TransferService::new(h.manager.clone()),
            exec,
            h,
        }
    }

    fn mutation(&self, dry_run: bool) -> Mutation {
        Mutation::new(id("a"), self.h.resources("a"), dry_run)
    }

    fn commands(&self) -> Vec<Vec<String>> {
        self.exec
            .recorded_calls()
            .into_iter()
            .filter_map(|call| match call {
                ExecStreamCall::Exec { command, .. } => Some(command),
                ExecStreamCall::Attach { .. } => None,
            })
            .collect()
    }
}

fn file() -> RemotePath {
    RemotePath::new("default", "web-0", "/etc/app.conf").in_container("app")
}

#[tokio::test]
async fn a_download_is_the_tar_stream() {
    let f = Fixture::new();
    f.exec
        .script()
        .exec
        .push_ok(ExecScript::new().stdout("tar-bytes"));
    let download = f
        .service
        .download(
            &id("a"),
            &file(),
            RemoteEntry::File,
            TransferOptions::default(),
        )
        .await
        .expect("download");
    assert_eq!(download.format, DownloadFormat::Tar);
    assert_eq!(collect(download.stream).await, b"tar-bytes");
    assert_eq!(f.commands()[0][4..6], ["tar", "-cf"]);
}

#[tokio::test]
async fn a_file_in_a_container_without_tar_falls_back_to_cat() {
    let f = Fixture::new();
    f.exec
        .script()
        .exec
        .push_ok(ExecScript::new().exit(Ok(ExitStatus::with_code(127))))
        .push_ok(ExecScript::new().stdout("raw file"));
    let download = f
        .service
        .download(
            &id("a"),
            &file(),
            RemoteEntry::File,
            TransferOptions::default(),
        )
        .await
        .expect("download");
    assert_eq!(download.format, DownloadFormat::Raw);
    assert_eq!(collect(download.stream).await, b"raw file");
    let commands = f.commands();
    assert_eq!(commands.len(), 2);
    assert_eq!(
        commands[1],
        vec!["cat".to_owned(), "/etc/app.conf".to_owned()]
    );
}

#[tokio::test]
async fn a_directory_in_a_container_without_tar_is_unsupported() {
    let f = Fixture::new();
    f.exec
        .script()
        .exec
        .push_ok(ExecScript::new().exit(Ok(ExitStatus::with_code(127))));
    let error = f
        .service
        .download(
            &id("a"),
            &file(),
            RemoteEntry::Directory,
            TransferOptions::default(),
        )
        .await
        .expect_err("no tar");
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(
        error
            .message()
            .contains("container app of pod default/web-0"),
        "{error}"
    );
    assert_eq!(f.commands().len(), 1, "no cat for a directory");
}

#[tokio::test]
async fn an_upload_needs_the_guards_mutation_and_feeds_tar() {
    let f = Fixture::new();
    let script = ExecScript::new();
    let capture = script.capture();
    f.exec.script().exec.push_ok(script);
    let dest = RemotePath::new("default", "web-0", "/srv");
    let receipt = f
        .service
        .upload(
            &f.mutation(false),
            &dest,
            chunked(b"archive", 3),
            TransferOptions::default(),
        )
        .await
        .expect("uploaded");
    assert_eq!(receipt.bytes, 7);
    assert!(!receipt.dry_run);
    assert_eq!(capture.stdin(), b"archive");
    assert!(capture.stdin_closed());
    assert_eq!(
        f.commands()[0],
        ["tar", "-xmf", "-", "-C", "/srv"].map(str::to_owned)
    );

    // A dry run sends nothing.
    let receipt = f
        .service
        .upload(
            &f.mutation(true),
            &dest,
            chunked(b"archive", 3),
            TransferOptions::default(),
        )
        .await
        .expect("dry run");
    assert!(receipt.dry_run);
    assert_eq!(f.commands().len(), 1);
}

#[tokio::test]
async fn a_cluster_that_is_not_open_is_an_error() {
    let f = Fixture::new();
    let error = f
        .service
        .download(
            &id("zz"),
            &file(),
            RemoteEntry::File,
            TransferOptions::default(),
        )
        .await
        .expect_err("not open");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    let mutation = Mutation::new(id("zz"), f.h.resources("a"), false);
    let error = f
        .service
        .upload(
            &mutation,
            &file(),
            futures::stream::empty().boxed(),
            TransferOptions::default(),
        )
        .await
        .expect_err("not open");
    assert_eq!(error.kind(), ErrorKind::NotFound);
}
