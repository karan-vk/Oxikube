//! `safe_unpack`: files land under the destination, escapes are refused, links are not created.

use std::path::{Path, PathBuf};

use bytes::Bytes;
use futures::{StreamExt as _, stream};
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::FsPort;
use oxikube_testkit::FakeFsPort;

use super::{archive_of, chunked};
use crate::transfer::{Skipped, safe_unpack};

fn dest() -> PathBuf {
    PathBuf::from("/downloads/pod")
}

/// A raw header for `name` of `kind` (`b'0'` file, `b'2'` symlink...) with `link`, bypassing
/// the tar crate's own path checks, as a hostile archive would.
fn raw_entry(name: &[u8], kind: u8, link: &[u8], data: &[u8]) -> Vec<u8> {
    let mut header = tar::Header::new_old();
    header.as_old_mut().name[..name.len()].copy_from_slice(name);
    header.as_old_mut().linkname[..link.len()].copy_from_slice(link);
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_entry_type(tar::EntryType::new(kind));
    header.set_cksum();
    let mut bytes = header.as_bytes().to_vec();
    bytes.extend_from_slice(data);
    bytes.resize(bytes.len().div_ceil(512) * 512, 0);
    bytes
}

fn end() -> Vec<u8> {
    vec![0; 1024]
}

#[tokio::test]
async fn files_land_under_the_destination_whatever_the_chunking() {
    let long = format!("deep/{}/file.txt", "d".repeat(150));
    let all: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
    let archive = archive_of(&[
        ("data/a.txt", b"alpha"),
        ("data/empty", b""),
        ("data/bytes.bin", &all),
        (long.as_str(), b"long name"),
    ]);
    for chunk in [1, 7, 512, 513, 100_000] {
        let fs = FakeFsPort::new();
        let report = safe_unpack(&fs, &dest(), chunked(&archive, chunk))
            .await
            .expect("unpacked");
        assert_eq!(report.files.len(), 4, "chunk {chunk}");
        assert_eq!(report.bytes, 5 + 5000 + 9);
        assert!(report.skipped.is_empty());
        assert_eq!(
            fs.read(&dest().join("data/a.txt")).await.expect("a"),
            b"alpha"
        );
        assert_eq!(fs.read(&dest().join("data/empty")).await.expect("e"), b"");
        assert_eq!(
            fs.read(&dest().join("data/bytes.bin")).await.expect("bin"),
            all,
            "every byte value survives"
        );
        assert_eq!(
            fs.read(&dest().join(&long)).await.expect("long"),
            b"long name"
        );
    }
}

#[tokio::test]
async fn pax_paths_are_honoured() {
    let mut builder = tar::Builder::new(Vec::new());
    builder
        .append_pax_extensions([("path", b"pax/named.txt".as_slice())])
        .expect("pax");
    let mut header = tar::Header::new_ustar();
    header.set_size(3);
    header.set_mode(0o644);
    builder
        .append_data(&mut header, "short", b"pax".as_slice())
        .expect("append");
    let archive = builder.into_inner().expect("finish");
    let fs = FakeFsPort::new();
    let report = safe_unpack(&fs, &dest(), chunked(&archive, 64))
        .await
        .expect("unpacked");
    assert_eq!(report.files, vec![PathBuf::from("pax/named.txt")]);
    assert_eq!(
        fs.read(&dest().join("pax/named.txt")).await.expect("pax"),
        b"pax"
    );
}

#[tokio::test]
async fn escaping_entries_stop_the_unpack() {
    for name in [b"../evil".as_slice(), b"/etc/cron.d/evil", b"ok/../../evil"] {
        let mut archive = archive_of(&[("first.txt", b"kept")]);
        archive.truncate(archive.len() - 1024);
        archive.extend(raw_entry(name, b'0', b"", b"pwned"));
        archive.extend(archive_of(&[("after.txt", b"never")]));
        let fs = FakeFsPort::new();
        let error = safe_unpack(&fs, &dest(), chunked(&archive, 100))
            .await
            .expect_err("refused");
        assert_eq!(error.kind(), ErrorKind::Validation);
        assert!(
            error.message().contains(&*String::from_utf8_lossy(name)),
            "{error}"
        );
        assert_eq!(
            fs.read(&dest().join("first.txt")).await.expect("kept"),
            b"kept"
        );
        assert!(fs.read(&dest().join("after.txt")).await.is_err());
        assert!(fs.read(Path::new("/downloads/evil")).await.is_err());
        assert!(fs.read(Path::new("/etc/cron.d/evil")).await.is_err());
    }
}

#[tokio::test]
async fn links_and_special_entries_are_never_created() {
    let mut archive = Vec::new();
    archive.extend(raw_entry(b"dir/inside", b'2', b"../file", b""));
    archive.extend(raw_entry(b"dir/outside", b'2', b"../../etc/passwd", b""));
    archive.extend(raw_entry(b"abs", b'2', b"/etc/shadow", b""));
    archive.extend(raw_entry(b"hard", b'1', b"dir/inside", b""));
    archive.extend(raw_entry(b"fifo", b'6', b"", b""));
    archive.extend(raw_entry(b"dir/", b'5', b"", b""));
    archive.extend(raw_entry(b"./", b'5', b"", b""));
    archive.extend(end());
    let fs = FakeFsPort::new();
    let report = safe_unpack(&fs, &dest(), chunked(&archive, 512))
        .await
        .expect("unpacked");
    assert!(report.files.is_empty());
    assert_eq!(
        report.skipped,
        vec![
            Skipped::Link("dir/inside".into()),
            Skipped::UnsafeLink("dir/outside".into()),
            Skipped::UnsafeLink("abs".into()),
            Skipped::HardLink("hard".into()),
            Skipped::Special("fifo".into()),
        ]
    );
    assert_eq!(report.skipped_count, 5);
    for name in ["dir/inside", "dir/outside", "abs", "hard", "fifo"] {
        assert!(
            fs.read(&dest().join(name)).await.is_err(),
            "{name} was created"
        );
    }
}

#[tokio::test]
async fn broken_input_is_an_error_and_abandons_the_file() {
    let archive = archive_of(&[("big.bin", &[9u8; 4096])]);
    // Truncated in the middle of the data.
    let fs = FakeFsPort::new();
    let error = safe_unpack(&fs, &dest(), chunked(&archive[..1500], 100))
        .await
        .expect_err("truncated");
    assert_eq!(error.kind(), ErrorKind::Validation);
    assert!(
        fs.read(&dest().join("big.bin")).await.is_err(),
        "no partial file"
    );

    // The download fails part-way: its error comes out, the file is abandoned.
    let fs = FakeFsPort::new();
    let first: Vec<_> = archive[..1500]
        .chunks(500)
        .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
        .chain([Err(OxiError::network("the connection dropped"))])
        .collect();
    let error = safe_unpack(&fs, &dest(), stream::iter(first).boxed())
        .await
        .expect_err("failed");
    assert_eq!(error.kind(), ErrorKind::Network);
    assert!(
        fs.read(&dest().join("big.bin")).await.is_err(),
        "no partial file"
    );

    // Not a tar archive at all (a shell's error message, say).
    let fs = FakeFsPort::new();
    let mut junk = b"sh: tar: not found\n".to_vec();
    junk.resize(512, b'x');
    let error = safe_unpack(&fs, &dest(), chunked(&junk, 512))
        .await
        .expect_err("junk");
    assert_eq!(error.kind(), ErrorKind::Validation);

    // An empty stream is an empty archive.
    let report = safe_unpack(&fs, &dest(), chunked(b"", 1))
        .await
        .expect("empty");
    assert!(report.files.is_empty());
}
