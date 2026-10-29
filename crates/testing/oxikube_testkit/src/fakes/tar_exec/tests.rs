//! `FakeTarExec` through the real transfer helpers (`ExecTransferExt`).

use bytes::Bytes;
use futures::executor::block_on;
use futures::{StreamExt as _, TryStreamExt as _, stream};
use oxikube_domain::{ErrorKind, OxiResult};
use oxikube_ports::exec::{ByteStream, ExecTransferExt, RemotePath, TransferOptions};
use tar::{Archive, Builder, Header};

use super::FakeTarExec;

fn remote(path: &str) -> RemotePath {
    RemotePath::new("demo", "web", path)
}

fn collect(stream: ByteStream) -> OxiResult<Vec<u8>> {
    block_on(stream.try_fold(Vec::new(), |mut all, chunk| async move {
        all.extend_from_slice(&chunk);
        Ok(all)
    }))
}

fn download(fake: &FakeTarExec, path: &str) -> OxiResult<Vec<u8>> {
    let stream = block_on(fake.tar_download(&remote(path), TransferOptions::default()))?;
    collect(stream)
}

fn entries(archive: &[u8]) -> Vec<(String, Vec<u8>, u32)> {
    let mut archive = Archive::new(archive);
    archive
        .entries()
        .unwrap()
        .map(|entry| {
            let mut entry = entry.unwrap();
            let name = entry.path().unwrap().to_string_lossy().into_owned();
            let mode = entry.header().mode().unwrap();
            let mut data = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut data).unwrap();
            (name, data, mode)
        })
        .collect()
}

fn archive_of(files: &[(&str, &[u8], u32)]) -> ByteStream {
    let mut builder = Builder::new(Vec::new());
    for (name, data, mode) in files {
        let mut header = Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(*mode);
        header.set_cksum();
        builder.append_data(&mut header, name, *data).unwrap();
    }
    let bytes = builder.into_inner().unwrap();
    stream::iter([Ok(Bytes::from(bytes))]).boxed()
}

fn upload(fake: &FakeTarExec, dir: &str, archive: ByteStream) -> OxiResult<u64> {
    block_on(fake.tar_upload(&remote(dir), archive, TransferOptions::default()))
}

#[test]
fn download_a_directory_is_a_real_tar_starting_at_the_name() {
    let fake = FakeTarExec::new()
        .with_file("/var/log/app.log", "hello")
        .with_file_mode("/var/log/run/job.sh", "#!/bin/sh", 0o755);
    let archive = download(&fake, "/var/log").unwrap();
    let names: Vec<_> = entries(&archive).into_iter().map(|(n, ..)| n).collect();
    assert_eq!(names, ["log/", "log/app.log", "log/run/", "log/run/job.sh"]);
    let job = entries(&archive).pop().unwrap();
    assert_eq!((job.1.as_slice(), job.2), (b"#!/bin/sh".as_slice(), 0o755));
}

#[test]
fn upload_then_download_round_trips_content_and_mode() {
    let fake = FakeTarExec::new().with_dir("/data");
    let sent = upload(
        &fake,
        "/data",
        archive_of(&[("a.bin", &[0, 255, 7], 0o600), ("sub/b.txt", b"b", 0o644)]),
    )
    .unwrap();
    assert!(sent > 0);
    assert_eq!(fake.file("/data/a.bin").unwrap(), [0, 255, 7]);
    assert_eq!(fake.mode("/data/a.bin"), Some(0o600));
    assert_eq!(fake.file("/data/sub/b.txt").unwrap(), b"b");
    let again = download(&fake, "/data/sub/b.txt").unwrap();
    assert_eq!(entries(&again)[0].0, "b.txt");
}

#[test]
fn a_big_download_is_chunked() {
    let fake = FakeTarExec::new().with_file("/big", vec![7_u8; 200_000]);
    let stream = block_on(fake.tar_download(&remote("/big"), TransferOptions::default())).unwrap();
    let chunks: Vec<Bytes> = block_on(stream.try_collect()).unwrap();
    assert!(chunks.len() > 1);
    assert_eq!(entries(&chunks.concat())[0].1.len(), 200_000);
}

#[test]
fn cat_download_is_the_file_without_an_archive() {
    let fake = FakeTarExec::new().with_file("/etc/motd", "welcome");
    let stream = block_on(fake.cat_download(&remote("/etc/motd"), TransferOptions::default()));
    assert_eq!(collect(stream.unwrap()).unwrap(), b"welcome");
}

#[test]
fn missing_tar_is_unsupported_and_cat_still_works() {
    let fake = FakeTarExec::new().without_tar().with_file("/f", "x");
    assert_eq!(
        download(&fake, "/f").unwrap_err().kind(),
        ErrorKind::Unsupported
    );
    assert_eq!(
        upload(&fake, "/", archive_of(&[("g", b"1", 0o644)]))
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
    let stream = block_on(fake.cat_download(&remote("/f"), TransferOptions::default()));
    assert_eq!(collect(stream.unwrap()).unwrap(), b"x");
}

#[test]
fn missing_cat_is_unsupported() {
    let fake = FakeTarExec::new().without_cat().with_file("/f", "x");
    let stream = block_on(fake.cat_download(&remote("/f"), TransferOptions::default()));
    assert_eq!(
        collect(stream.unwrap()).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
}

#[test]
fn a_missing_path_is_not_found() {
    let fake = FakeTarExec::new();
    assert_eq!(
        download(&fake, "/nope").unwrap_err().kind(),
        ErrorKind::NotFound
    );
    assert_eq!(
        upload(&fake, "/nope", archive_of(&[("g", b"1", 0o644)]))
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
}

#[test]
fn permission_denied_and_read_only_are_forbidden() {
    let fake = FakeTarExec::new()
        .with_file("/root/secret", "s")
        .with_dir("/tmp")
        .deny("/root");
    assert_eq!(
        download(&fake, "/root/secret").unwrap_err().kind(),
        ErrorKind::Forbidden
    );
    assert_eq!(
        upload(&fake, "/root", archive_of(&[("g", b"1", 0o644)]))
            .unwrap_err()
            .kind(),
        ErrorKind::Forbidden
    );
    let denied = FakeTarExec::new().with_dir("/ro").deny("/ro");
    assert_eq!(
        upload(&denied, "/ro", archive_of(&[("g", b"1", 0o644)]))
            .unwrap_err()
            .kind(),
        ErrorKind::Forbidden
    );
    let read_only = FakeTarExec::new().with_dir("/ro").read_only();
    assert_eq!(
        upload(&read_only, "/ro", archive_of(&[("g", b"1", 0o644)]))
            .unwrap_err()
            .kind(),
        ErrorKind::Forbidden
    );
    assert!(read_only.file("/ro/g").is_none());
}

#[test]
fn calls_record_the_container_and_command() {
    let fake = FakeTarExec::new().with_file("/f", "x");
    let source = remote("/f").in_container("sidecar");
    let _stream = block_on(fake.tar_download(&source, TransferOptions::default())).unwrap();
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].container.as_deref(), Some("sidecar"));
    assert!(calls[0].command.contains(&"tar".to_owned()));
}
