//! Kind integration for E20-S06, the exec tar helpers (`ExecTransferExt`) over `KubeExec` on a
//! busybox pod: a 100 MB download checked against `sha256sum` in the pod (with throughput, progress
//! and memory numbers), an upload checked for content and mode, every byte value round-tripping,
//! a cancel that leaves no `tar` running in the pod, the `cat` fallback, and a container without
//! `tar`. Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use std::io::Read as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures::{StreamExt as _, stream};
use oxikube_domain::ErrorKind;
use oxikube_kube::KubeExec;
use oxikube_ports::exec::ByteStream;
use oxikube_ports::{
    CancellationToken, ExecOptions, ExecStreamPort, ExecTransferExt, RemotePath, TransferOptions,
    TransferProgress,
};
use oxikube_testkit::integration::TestNamespace;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use common::exec::{create_shell_less, create_sleeper, read_all};
use common::portforward::wait_ready;
use common::{DEADLINE, wait_until};

const POD: &str = "box";
const MIB: u64 = 1024 * 1024;

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| (*p).to_owned()).collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Runs `script` with `sh -c` in the pod's `main` container; returns stdout, asserting exit 0.
async fn sh(exec: &KubeExec, namespace: &str, script: &str) -> String {
    let options = ExecOptions {
        stderr: false,
        ..ExecOptions::default()
    }
    .container("main");
    let mut session = exec
        .exec_session(namespace, POD, &argv(&["sh", "-c", script]), &options)
        .await
        .expect("exec");
    let out = read_all(session.stdout.as_mut().expect("stdout")).await;
    let status = session.status.await.expect("status");
    assert!(status.is_success(), "`{script}` failed: {status:?}");
    String::from_utf8_lossy(&out).into_owned()
}

/// A busybox pod in a fresh namespace, and the adapter.
async fn busybox() -> Option<(TestNamespace, KubeExec)> {
    let kind = common::kind().await?;
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_sleeper(&client, ns.name(), POD).await;
    wait_ready(&client, ns.name(), POD).await;
    Some((ns, KubeExec::new((*client).clone())))
}

fn path(ns: &TestNamespace, path: &str) -> RemotePath {
    RemotePath::new(ns.name(), POD, path).in_container("main")
}

/// This process's resident memory in KiB, from `ps` (macOS and Linux).
fn rss_kib() -> Option<u64> {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// Whether a `tar` process is running in the pod.
async fn tar_running(exec: &KubeExec, namespace: &str) -> bool {
    // `[t]ar` keeps the grep itself out of the match.
    let out = sh(exec, namespace, "ps -o pid,args | grep '[t]ar -' || true").await;
    !out.trim().is_empty()
}

/// Streams the single-file archive `stream`, hashing the entry's `size` data bytes (after the
/// 512-byte header) without holding them.
async fn hash_single_entry(mut stream: ByteStream, size: u64) -> (String, u64, usize) {
    let mut hasher = Sha256::new();
    let mut offset = 0u64;
    let mut largest = 0usize;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.expect("a chunk");
        largest = largest.max(chunk.len());
        let start = offset;
        offset += chunk.len() as u64;
        let from = 512u64.saturating_sub(start).min(chunk.len() as u64);
        let to = (512 + size).saturating_sub(start).min(chunk.len() as u64);
        if from < to {
            hasher.update(&chunk[from as usize..to as usize]);
        }
    }
    (hex(&hasher.finalize()), offset, largest)
}

#[tokio::test]
async fn a_100_mb_download_matches_the_pods_checksum() {
    let Some((ns, exec)) = busybox().await else {
        return;
    };
    let size = 100 * MIB;
    let made = sh(
        &exec,
        ns.name(),
        "dd if=/dev/urandom of=/tmp/big bs=1048576 count=100 2>/dev/null && sha256sum /tmp/big",
    )
    .await;
    let expected = made.split_whitespace().next().expect("a hash").to_owned();

    let reports = Arc::new(Mutex::new(Vec::<(Instant, TransferProgress)>::new()));
    let sink = reports.clone();
    let options = TransferOptions {
        total_hint: Some(size),
        progress: Some(Arc::new(move |progress| {
            sink.lock().push((Instant::now(), progress));
        })),
        ..TransferOptions::default()
    };
    let before = rss_kib();
    let peak = Arc::new(AtomicUsize::new(0));
    let sampler_peak = peak.clone();
    let sampler = tokio::spawn(async move {
        loop {
            if let Some(rss) = rss_kib() {
                sampler_peak.fetch_max(rss as usize, Ordering::Relaxed);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });
    let started = Instant::now();
    let archive = exec
        .tar_download(&path(&ns, "/tmp/big"), options)
        .await
        .expect("download");
    let (got, archive_bytes, largest_chunk) = hash_single_entry(archive, size).await;
    let elapsed = started.elapsed();
    sampler.abort();
    assert_eq!(
        got, expected,
        "the downloaded file matches sha256sum in the pod"
    );
    assert!(archive_bytes >= size + 512, "{archive_bytes}");

    let reports = reports.lock();
    let last = reports.last().expect("progress").1;
    assert_eq!(last.bytes, archive_bytes);
    assert_eq!(last.total, Some(size));
    let per_second = reports.len() as f64 / elapsed.as_secs_f64().max(0.1);
    let closest = reports
        .windows(2)
        .take(reports.len().saturating_sub(2))
        .map(|pair| pair[1].0 - pair[0].0)
        .min();
    if let Some(closest) = closest {
        assert!(
            closest >= Duration::from_millis(95),
            "reports {closest:?} apart"
        );
    }
    let peak_over = match (before, peak.load(Ordering::Relaxed)) {
        (Some(before), peak) if peak > 0 => {
            format!("{} KiB", peak as u64 - before.min(peak as u64))
        }
        _ => "unknown".to_owned(),
    };
    eprintln!(
        "exec tar download of 100 MiB: {elapsed:?} ({:.1} MiB/s), {} progress reports \
         ({per_second:.1}/s), largest chunk {largest_chunk} B, peak RSS over baseline {peak_over}",
        size as f64 / MIB as f64 / elapsed.as_secs_f64(),
        reports.len()
    );
    assert!(
        largest_chunk <= 64 * 1024,
        "chunks stay small: {largest_chunk}"
    );
}

/// A one-file archive of `contents` named `name` with `mode`, as the upload's source stream.
fn one_file_archive(name: &str, contents: &[u8], mode: u32) -> ByteStream {
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(contents.len() as u64);
    header.set_mode(mode);
    builder
        .append_data(&mut header, name, contents)
        .expect("append");
    let archive = builder.into_inner().expect("finish");
    let chunks: Vec<_> = archive
        .chunks(32 * 1024)
        .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
        .collect();
    stream::iter(chunks).boxed()
}

#[tokio::test]
async fn an_upload_lands_with_its_content_and_mode() {
    let Some((ns, exec)) = busybox().await else {
        return;
    };
    sh(&exec, ns.name(), "mkdir -p /tmp/in").await;
    let contents: Vec<u8> = (0..300_000u32)
        .map(|i| (i.wrapping_mul(131) % 256) as u8)
        .collect();
    let sent = exec
        .tar_upload(
            &path(&ns, "/tmp/in"),
            one_file_archive("up.bin", &contents, 0o751),
            TransferOptions::default(),
        )
        .await
        .expect("upload");
    assert!(sent >= contents.len() as u64);
    let remote = sh(
        &exec,
        ns.name(),
        "sha256sum /tmp/in/up.bin; stat -c %a /tmp/in/up.bin",
    )
    .await;
    let mut lines = remote.lines();
    let hash = lines
        .next()
        .and_then(|l| l.split_whitespace().next())
        .expect("hash");
    assert_eq!(hash, hex(&Sha256::digest(&contents)));
    assert_eq!(lines.next().map(str::trim), Some("751"), "{remote}");

    // Into a directory that does not exist: a typed error, not a hang.
    let error = exec
        .tar_upload(
            &path(&ns, "/tmp/missing"),
            one_file_archive("x", b"x", 0o644),
            TransferOptions::default(),
        )
        .await
        .expect_err("no such directory");
    assert_eq!(error.kind(), ErrorKind::NotFound, "{error}");
}

#[tokio::test]
async fn every_byte_value_round_trips_and_cat_reads_one_file() {
    let Some((ns, exec)) = busybox().await else {
        return;
    };
    sh(&exec, ns.name(), "mkdir -p /tmp/rt").await;
    let all: Vec<u8> = (0..=255u8).cycle().take(256 * 1024).collect();
    exec.tar_upload(
        &path(&ns, "/tmp/rt"),
        one_file_archive("bytes.bin", &all, 0o644),
        TransferOptions::default(),
    )
    .await
    .expect("upload");

    let archive: Vec<u8> = exec
        .tar_download(&path(&ns, "/tmp/rt/bytes.bin"), TransferOptions::default())
        .await
        .expect("download")
        .map(|chunk| chunk.expect("chunk").to_vec())
        .concat()
        .await;
    let mut entries = tar::Archive::new(archive.as_slice());
    let mut entry = entries
        .entries()
        .expect("entries")
        .next()
        .expect("one")
        .expect("ok");
    assert_eq!(entry.path().expect("path").to_str(), Some("bytes.bin"));
    let mut back = Vec::new();
    entry.read_to_end(&mut back).expect("read");
    assert!(back == all, "the bytes changed on the way");

    let raw: Vec<u8> = exec
        .cat_download(&path(&ns, "/tmp/rt/bytes.bin"), TransferOptions::default())
        .await
        .expect("cat")
        .map(|chunk| chunk.expect("chunk").to_vec())
        .concat()
        .await;
    assert!(raw == all, "cat is binary-safe without a TTY");

    let missing = exec
        .tar_download(&path(&ns, "/tmp/nope"), TransferOptions::default())
        .await
        .expect("open")
        .collect::<Vec<_>>()
        .await;
    let error = missing
        .into_iter()
        .last()
        .expect("an item")
        .expect_err("missing");
    assert_eq!(error.kind(), ErrorKind::NotFound, "{error}");
}

#[tokio::test]
async fn a_cancel_stops_the_remote_tar() {
    let Some((ns, exec)) = busybox().await else {
        return;
    };
    sh(
        &exec,
        ns.name(),
        "dd if=/dev/zero of=/tmp/big bs=1048576 count=64 2>/dev/null",
    )
    .await;
    let cancel = CancellationToken::new();
    let options = TransferOptions {
        cancel: cancel.clone(),
        ..TransferOptions::default()
    };
    let mut archive = exec
        .tar_download(&path(&ns, "/tmp/big"), options)
        .await
        .expect("download");
    archive.next().await.expect("a chunk").expect("ok");
    assert!(
        tar_running(&exec, ns.name()).await,
        "tar waits on the unread stream"
    );
    cancel.cancel();
    let error = archive
        .next()
        .await
        .expect("an item")
        .expect_err("cancelled");
    assert_eq!(error.kind(), ErrorKind::Conflict);
    drop(archive);
    wait_until("the remote tar exits", DEADLINE, || async {
        (!tar_running(&exec, ns.name()).await).then_some(())
    })
    .await;

    // An upload cancelled while its archive is still coming.
    sh(&exec, ns.name(), "mkdir -p /tmp/up").await;
    let cancel = CancellationToken::new();
    let options = TransferOptions {
        cancel: cancel.clone(),
        ..TransferOptions::default()
    };
    let header = one_file_archive("slow.bin", &[0u8; 4096], 0o644)
        .take(1)
        .chain(stream::pending())
        .boxed();
    let target = path(&ns, "/tmp/up");
    let upload = exec.tar_upload(&target, header, options);
    let canceller = async {
        wait_until("the remote tar starts", DEADLINE, || async {
            tar_running(&exec, ns.name()).await.then_some(())
        })
        .await;
        cancel.cancel();
    };
    let (outcome, ()) = futures::join!(upload, canceller);
    assert_eq!(outcome.expect_err("cancelled").kind(), ErrorKind::Conflict);
    wait_until("the remote tar exits", DEADLINE, || async {
        (!tar_running(&exec, ns.name()).await).then_some(())
    })
    .await;
}

#[tokio::test]
async fn a_container_without_tar_is_unsupported() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_shell_less(&client, ns.name(), POD).await;
    wait_ready(&client, ns.name(), POD).await;
    let exec = KubeExec::new((*client).clone());
    let outcome = match exec
        .tar_download(&path(&ns, "/etc"), TransferOptions::default())
        .await
    {
        Err(error) => error,
        Ok(stream) => stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .last()
            .expect("an item")
            .expect_err("no tar"),
    };
    assert_eq!(outcome.kind(), ErrorKind::Unsupported, "{outcome}");
    assert!(outcome.message().contains("container main"), "{outcome}");
}
