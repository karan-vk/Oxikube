//! E15-S02 bench: 100 simultaneous connections through one forward to an in-memory echo pod.
//!
//! Run with numbers:
//! `cargo test -p oxikube_kube --release --lib multiplex_bench -- --ignored --nocapture`
//! It reports connection setup per second, MB/s through the forward and RSS per open
//! connection. The fake pod side is a 64 KiB in-memory duplex, so the numbers measure the
//! listener, the bridge and the counters, not a cluster.

use std::time::Instant;

use oxikube_domain::ForwardPort;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::fakes::{FakeCluster, FakeConnector, eventually, pod, spec, start};

const CONNS: usize = 100;
const PAYLOAD: usize = 4 * 1024 * 1024;

/// Resident set size of this process in KiB (`ps`, so it works on macOS and Linux).
fn rss_kib() -> Option<u64> {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "bench: run with --release -- --ignored --nocapture"]
async fn bench_100_concurrent_connections() {
    let cluster = FakeCluster::new(None, vec![pod("web-1", 1)]);
    let connector = FakeConnector::new([]);
    let handle = start(
        &cluster,
        &connector,
        &spec("Pod", "web-1", ForwardPort::Number(80)),
    )
    .await
    .expect("start");
    let addr = handle.local_addr();
    let rss_before = rss_kib();

    // Setup: open every connection and prove each reached the pod with one echoed byte.
    let began = Instant::now();
    let opened = futures::future::join_all((0..CONNS).map(|_| async move {
        let mut socket = TcpStream::connect(addr).await.expect("connect");
        socket.write_all(b"x").await.expect("write");
        let mut byte = [0u8];
        socket.read_exact(&mut byte).await.expect("echo");
        socket
    }))
    .await;
    let setup = began.elapsed();
    let rss_open = rss_kib();

    // Throughput: every connection pushes PAYLOAD bytes and reads them back, all at once.
    let began = Instant::now();
    let transfers = opened.into_iter().map(|mut socket| {
        tokio::spawn(async move {
            let payload = vec![7u8; PAYLOAD];
            let (mut read, mut write) = socket.split();
            let send = async {
                write.write_all(&payload).await.expect("write");
                write.shutdown().await.expect("half-close");
            };
            let mut back = 0usize;
            let recv = async {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match read.read(&mut buf).await.expect("read") {
                        0 => break,
                        n => back += n,
                    }
                }
            };
            tokio::join!(send, recv);
            assert_eq!(back, PAYLOAD);
        })
    });
    for transfer in futures::future::join_all(transfers).await {
        transfer.expect("transfer");
    }
    let transfer = began.elapsed();

    let stats = handle.stats().clone();
    eventually("connections to close", || {
        stats.snapshot().active_connections == 0
    })
    .await;
    let s = stats.snapshot();
    assert_eq!(s.total_connections, CONNS as u64);
    assert_eq!(s.bytes_in, (CONNS * PAYLOAD + CONNS) as u64);

    let moved_mb = (2 * CONNS * PAYLOAD) as f64 / (1024.0 * 1024.0);
    eprintln!(
        "E15-S02 bench: {CONNS} conns set up in {setup:?} ({:.0} conn/s); \
         {moved_mb:.0} MiB through the forward in {transfer:?} ({:.0} MiB/s)",
        CONNS as f64 / setup.as_secs_f64(),
        moved_mb / transfer.as_secs_f64(),
    );
    if let (Some(before), Some(open)) = (rss_before, rss_open) {
        eprintln!(
            "E15-S02 bench: RSS {before} KiB idle, {open} KiB with {CONNS} open \
             (~{:.1} KiB per connection, incl. the fake pod side)",
            open.saturating_sub(before) as f64 / CONNS as f64
        );
    }
}
