//! "Tail in terminal" (E08-S08) against a real cluster: the argv the log view builds, run by the
//! terminal's local PTY with the cluster's environment (a merged `KUBECONFIG` of the one context),
//! really streams the pod's log through `kubectl logs -f`.
//!
//! Needs `OXIKUBE_TEST_CONTEXT` (the kind context, e.g. `kind-oxikube`) and kubectl on this
//! machine; without either the test says so and passes. It makes its own namespace with one small
//! busybox pod that echoes a line a second, and deletes the namespace afterwards. The cluster is
//! shared with other tests: nothing here touches anything outside that namespace.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use futures::StreamExt as _;
use oxikube_app::logs::kubectl::{KubectlLookup as _, KubectlTail, PathLookup, TailTarget};
use oxikube_domain::ids::ContextName;
use oxikube_ports::LogOptions;
use oxikube_ports::exec::{BackendEvent, TerminalBackend as _};
use oxikube_terminal::backend::local::{ClusterEnv, LocalPty, LocalPtyOptions};

const WAIT: Duration = Duration::from_secs(90);

/// A namespace that is deleted (without waiting) when the test ends, pass or fail.
struct Namespace {
    kubectl: PathBuf,
    context: String,
    name: String,
}

impl Namespace {
    fn kubectl(&self, args: &[&str]) -> std::process::Output {
        Command::new(&self.kubectl)
            .arg(format!("--context={}", self.context))
            .args(args)
            .output()
            .expect("kubectl runs")
    }
}

impl Drop for Namespace {
    fn drop(&mut self) {
        self.kubectl(&["delete", "namespace", &self.name, "--wait=false"]);
    }
}

fn kubeconfig_file() -> PathBuf {
    std::env::var_os("KUBECONFIG")
        .and_then(|paths| std::env::split_paths(&paths).next())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".kube/config")))
        .expect("a kubeconfig")
}

#[tokio::test]
async fn kubectl_logs_follows_a_pod_with_the_cluster_environment() {
    let Ok(context) = std::env::var("OXIKUBE_TEST_CONTEXT") else {
        eprintln!("skipped: OXIKUBE_TEST_CONTEXT is not set");
        return;
    };
    let Some(kubectl) = PathLookup::system().find() else {
        eprintln!("skipped: kubectl is not installed");
        return;
    };
    let suffix = std::process::id() ^ (jiff::Timestamp::now().as_millisecond() as u32);
    let namespace = Namespace {
        kubectl: kubectl.clone(),
        context: context.clone(),
        name: format!("oxikube-e08s08-{suffix:x}"),
    };
    let created = namespace.kubectl(&["create", "namespace", &namespace.name]);
    assert!(created.status.success(), "{created:?}");
    let script = "i=0; while true; do echo tail-line-$i; i=$((i+1)); sleep 1; done";
    let run = namespace.kubectl(&[
        "run",
        "ticker",
        "--image=busybox:1.36",
        "--restart=Never",
        "--namespace",
        &namespace.name,
        "--",
        "sh",
        "-c",
        script,
    ]);
    assert!(run.status.success(), "{run:?}");
    let ready = namespace.kubectl(&[
        "wait",
        "--for=condition=Ready",
        "pod/ticker",
        "--namespace",
        &namespace.name,
        "--timeout=120s",
    ]);
    assert!(ready.status.success(), "the pod is ready: {ready:?}");

    // What the log view would run for this pod: the tail, following, with timestamps shown.
    let request = KubectlTail {
        context: context.clone(),
        namespace: namespace.name.clone(),
        target: TailTarget::Pod("ticker".into()),
        options: LogOptions {
            follow: true,
            timestamps: true,
            tail_lines: Some(1_000),
            ..LogOptions::default()
        },
        timestamps: true,
        max_log_requests: 20,
    };
    let args = request.argv().expect("a valid command");
    assert_eq!(&args[..2], ["logs", "-f"]);

    // The terminal's own start: the program with its arguments, the cluster in its environment.
    let env = ClusterEnv::new(ContextName::new(context), vec![kubeconfig_file()])
        .in_namespace(namespace.name.clone());
    let pty = LocalPty::spawn(LocalPtyOptions {
        shell: Some(kubectl.to_string_lossy().into_owned()),
        args,
        cluster: Some(env),
        ..LocalPtyOptions::default()
    })
    .expect("kubectl starts on a PTY");

    let mut output = String::new();
    let mut stream = pty.output_stream();
    let read = async {
        while let Some(event) = stream.next().await {
            match event {
                BackendEvent::Output(bytes) => {
                    output.push_str(&String::from_utf8_lossy(&bytes));
                    // A line the pod wrote after kubectl attached: it is following, not replaying.
                    if output.contains("tail-line-2") {
                        return true;
                    }
                }
                BackendEvent::Exited(_) => return false,
                BackendEvent::Error(error) => panic!("backend error: {error}"),
            }
        }
        false
    };
    let followed = tokio::time::timeout(WAIT, read).await.expect("in time");
    assert!(followed, "kubectl streamed the pod's lines; got {output:?}");
    // Timestamps were asked for: each line starts with an RFC 3339 time.
    assert!(
        output.contains("20") && output.contains('T') && output.contains('Z'),
        "{output:?}"
    );
    drop(stream);
    pty.kill().await.ok();
}
