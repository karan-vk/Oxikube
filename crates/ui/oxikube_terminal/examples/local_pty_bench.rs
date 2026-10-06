//! Numbers for E09-S02 (docs/PERFORMANCE.md): how long a local shell takes to open and how fast
//! its output flows.
//!
//! `cargo run --release -p oxikube_terminal --example local_pty_bench [-- <shell>]`
//!
//! - open to first output: `LocalPty::spawn` until the first byte (an interactive shell's first
//!   prompt; the epic budget is 150 ms with the cluster environment).
//! - cluster env: the cost of cutting and writing the merged kubeconfig.
//! - throughput: `yes` read for 2 seconds through the bounded output queue.

#![allow(clippy::print_stdout)]

use std::time::{Duration, Instant};

use futures::StreamExt as _;
use oxikube_domain::ids::ContextName;
use oxikube_ports::exec::{BackendEvent, TerminalBackend as _};
use oxikube_terminal::backend::local::{ClusterEnv, LocalPty, LocalPtyOptions};

const KUBECONFIG: &str = "contexts:\n- name: bench\n  context: {cluster: k, user: u}\nclusters:\n- name: k\n  cluster: {server: 'https://k.example'}\nusers:\n- name: u\n  user: {token: bench-token}\n";

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

async fn first_output(options: LocalPtyOptions) -> (Duration, Duration) {
    let started = Instant::now();
    let pty = LocalPty::spawn(options).expect("spawn");
    let spawned = started.elapsed();
    let mut stream = pty.output_stream();
    while let Some(event) = stream.next().await {
        if matches!(event, BackendEvent::Output(_)) {
            break;
        }
    }
    (spawned, started.elapsed())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let shell = std::env::args().nth(1).unwrap_or_else(|| "/bin/sh".into());
    let dir = std::env::temp_dir().join(format!("oxikube-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("kubeconfig");
    std::fs::write(&file, KUBECONFIG).unwrap();
    let cluster = || ClusterEnv::new(ContextName::new("bench"), vec![file.clone()]);
    let interactive = |cluster: Option<ClusterEnv>| LocalPtyOptions {
        shell: Some(shell.clone()),
        cluster,
        ..LocalPtyOptions::default()
    };

    for (label, with_cluster) in [("plain shell", false), ("cluster shell", true)] {
        let mut spawn = Vec::new();
        let mut total = Vec::new();
        for _ in 0..30 {
            let options = interactive(with_cluster.then(cluster));
            let (spawned, first) = first_output(options).await;
            spawn.push(spawned);
            total.push(first);
        }
        spawn.sort();
        total.sort();
        println!(
            "{label} ({shell}): spawn p50 {:?} p95 {:?}; open-to-first-output p50 {:?} p95 {:?}",
            percentile(&spawn, 0.5),
            percentile(&spawn, 0.95),
            percentile(&total, 0.5),
            percentile(&total, 0.95),
        );
    }

    let started = Instant::now();
    for _ in 0..100 {
        cluster().prepare().unwrap();
    }
    println!("cluster env prepare: {:?} each", started.elapsed() / 100);

    let options = LocalPtyOptions {
        shell: Some("/bin/sh".into()),
        args: vec!["-c".into(), "yes".into()],
        ..LocalPtyOptions::default()
    };
    let pty = LocalPty::spawn(options).unwrap();
    let mut stream = pty.output_stream();
    let started = Instant::now();
    let (mut bytes, mut chunks) = (0usize, 0usize);
    while started.elapsed() < Duration::from_secs(2) {
        if let Some(BackendEvent::Output(chunk)) = stream.next().await {
            bytes += chunk.len();
            chunks += 1;
        }
    }
    let secs = started.elapsed().as_secs_f64();
    println!(
        "throughput: {:.0} MiB/s in {chunks} chunks ({:.0} bytes/chunk)",
        bytes as f64 / secs / 1048576.0,
        bytes as f64 / chunks as f64
    );
    pty.kill().await.unwrap();
    std::fs::remove_dir_all(&dir).ok();
}
