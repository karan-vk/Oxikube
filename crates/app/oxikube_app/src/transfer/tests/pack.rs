//! `tar_file_stream`: one file as a tar stream, checked by the tar crate and by `safe_unpack`.

use std::io::Read as _;
use std::path::PathBuf;

use oxikube_domain::ErrorKind;
use oxikube_ports::FsPort;
use oxikube_testkit::FakeFsPort;

use super::{chunked, collect};
use crate::transfer::{safe_unpack, tar_file_stream};

#[tokio::test]
async fn a_file_becomes_a_tar_any_tar_reads() {
    let all: Vec<u8> = (0..=255u8).cycle().take(70_000).collect();
    let name = format!("{}.bin", "n".repeat(120));
    let stream =
        tar_file_stream(&name, 0o755, all.len() as u64, chunked(&all, 32 * 1024)).expect("stream");
    let archive = collect(stream).await;
    assert_eq!(archive.len() % 512, 0);

    let mut entries = tar::Archive::new(archive.as_slice());
    let mut entries = entries.entries().expect("entries");
    let mut entry = entries.next().expect("one entry").expect("ok");
    assert_eq!(entry.path().expect("path").to_str(), Some(name.as_str()));
    assert_eq!(entry.header().mode().expect("mode"), 0o755);
    let mut body = Vec::new();
    entry.read_to_end(&mut body).expect("read");
    assert_eq!(body, all);
    drop(entry);
    assert!(entries.next().is_none());

    // And it round-trips through the safe unpack.
    let fs = FakeFsPort::new();
    let report = safe_unpack(&fs, &PathBuf::from("/x"), chunked(&archive, 1000))
        .await
        .expect("unpack");
    assert_eq!(report.files, vec![PathBuf::from(&name)]);
    assert_eq!(
        fs.read(&PathBuf::from("/x").join(&name))
            .await
            .expect("read"),
        all
    );
}

#[tokio::test]
async fn bad_names_and_changed_files_are_errors() {
    for name in ["", ".", "..", "a/b", "a\0b"] {
        let error = tar_file_stream(name, 0o644, 0, chunked(b"", 1))
            .err()
            .expect("refused");
        assert_eq!(error.kind(), ErrorKind::Validation, "{name:?}");
    }
    let short = tar_file_stream("f", 0o644, 10, chunked(b"12345", 2)).expect("stream");
    let items: Vec<_> = futures::StreamExt::collect(short).await;
    assert!(
        items.last().expect("an item").is_err(),
        "a short file fails"
    );

    let long = tar_file_stream("f", 0o644, 2, chunked(b"12345", 2)).expect("stream");
    let items: Vec<_> = futures::StreamExt::collect(long).await;
    assert!(items.iter().any(Result::is_err), "a grown file fails");
}
