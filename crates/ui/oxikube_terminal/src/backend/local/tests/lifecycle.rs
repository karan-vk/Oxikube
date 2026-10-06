//! Kill, drop, exit handling and cleanup.

use std::time::{Duration, Instant};

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::ContextName;

use super::super::unix::process_exists;
use super::*;

async fn gone(pid: u32) -> bool {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if !process_exists(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test]
async fn kill_ends_the_child_and_the_stream() {
    let pty = LocalPty::spawn(sh("echo ready; sleep 1000")).unwrap();
    let mut stream = pty.output_stream();
    read_until(&mut stream, |out| out.contains("ready")).await;
    let pid = pty.process_id().unwrap();

    pty.kill().await.unwrap();
    let run = read_until(&mut stream, |_| false).await;
    assert_eq!(run.status, Some(ExitStatus::killed_by("KILL")));
    assert!(run.ended);
    assert!(gone(pid).await, "the shell is reaped");

    // Idempotent, and the session refuses further input.
    pty.kill().await.unwrap();
    assert_eq!(
        pty.write(b"x").await.err().unwrap().kind(),
        ErrorKind::Conflict
    );
    assert_eq!(
        pty.resize(TerminalSize::new(10, 10))
            .await
            .err()
            .unwrap()
            .kind(),
        ErrorKind::Conflict
    );
}

#[tokio::test]
async fn kill_takes_the_whole_process_group() {
    let pty = LocalPty::spawn(sh("sleep 1000 & echo child=$!; wait")).unwrap();
    let mut stream = pty.output_stream();
    let run = read_until(&mut stream, |out| {
        out.contains("child=") && out.contains('\n')
    })
    .await;
    let child: u32 = run
        .output
        .split("child=")
        .nth(1)
        .and_then(|rest| rest.trim().split_whitespace().next())
        .and_then(|pid| pid.parse().ok())
        .unwrap_or_else(|| panic!("no pid in {:?}", run.output));
    assert!(process_exists(child));

    pty.kill().await.unwrap();
    assert!(gone(child).await, "the background job died with the shell");
}

#[tokio::test]
async fn dropping_the_backend_kills_the_shell() {
    let pty = LocalPty::spawn(sh("sleep 1000")).unwrap();
    let pid = pty.process_id().unwrap();
    assert!(process_exists(pid));
    drop(pty);
    assert!(gone(pid).await);
}

#[tokio::test]
async fn a_signal_death_reports_the_signal() {
    let pty = LocalPty::spawn(sh("kill -TERM $$")).unwrap();
    let run = read_to_exit(&pty).await;
    assert_eq!(run.status, Some(ExitStatus::killed_by("TERM")));
}

#[tokio::test]
async fn a_leftover_background_process_does_not_delay_the_exit() {
    // The shell exits at once; `sleep` keeps the PTY open for ten seconds.
    let started = Instant::now();
    let pty = LocalPty::spawn(sh("sleep 10 & echo bye; exit 0")).unwrap();
    let run = read_to_exit(&pty).await;
    assert!(run.output.contains("bye"));
    assert_eq!(run.status, Some(ExitStatus::success()));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    // After the exit the session is closed for input.
    assert_eq!(
        pty.write(b"x").await.err().unwrap().kind(),
        ErrorKind::Conflict
    );
}

#[tokio::test]
async fn a_flood_is_throttled_and_still_killable() {
    let pty = LocalPty::spawn(sh("yes")).unwrap();
    let mut stream = pty.output_stream();
    // Nobody reads for a while (the bound itself is tested in `backpressure`); a flood must
    // still arrive in full once the consumer is there, and the shell must stay killable.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut seen = 0usize;
    let run = read_until(&mut stream, |out| {
        seen = out.len();
        seen > 4 * 1024 * 1024
    })
    .await;
    assert!(seen > 4 * 1024 * 1024, "{}", run.output.len());
    pty.kill().await.unwrap();
    let end = read_until(&mut stream, |_| false).await;
    assert_eq!(end.status, Some(ExitStatus::killed_by("KILL")));
}

fn cluster_file(dir: &std::path::Path, token: &str) -> std::path::PathBuf {
    let path = dir.join("kubeconfig");
    std::fs::write(
        &path,
        format!(
            "contexts:\n- name: c1\n  context: {{cluster: k, user: u}}\nclusters:\n- name: k\n  cluster: {{server: 'https://k.example'}}\nusers:\n- name: u\n  user: {{token: {token}}}\n"
        ),
    )
    .unwrap();
    path
}

#[tokio::test]
async fn the_cluster_environment_is_set_and_the_file_is_gone_after_the_exit() {
    use std::os::unix::fs::PermissionsExt as _;
    let src = tempfile::tempdir().unwrap();
    // Unique values: the runtime directory is shared with the tests running in parallel.
    let (token, namespace) = ("tok-4f9c1d-never-on-disk", "ns-7d2e-payments");
    let cluster = ClusterEnv::new(
        ContextName::new("c1"),
        vec![cluster_file(src.path(), token)],
    )
    .in_namespace(namespace);
    let script = r#"printf 'KC=%s\nCTX=%s\nNS=%s\nPATH=%s\n' "$KUBECONFIG" "$KUBE_CONTEXT" "$OXIKUBE_NAMESPACE" "$PATH"; read _"#;
    let pty = LocalPty::spawn(sh(script).for_cluster(cluster)).unwrap();
    let mut stream = pty.output_stream();
    let run = read_until(&mut stream, |out| {
        out.contains("PATH=") && out.ends_with('\n')
    })
    .await;
    let field = |name: &str| -> String {
        run.output
            .lines()
            .find_map(|l| l.trim_end().strip_prefix(&format!("{name}=")))
            .unwrap_or_else(|| panic!("no {name} in {:?}", run.output))
            .to_owned()
    };
    assert_eq!(field("CTX"), "c1");
    assert_eq!(field("NS"), namespace);
    assert_eq!(Some(field("PATH")), std::env::var("PATH").ok());

    let kubeconfig = std::path::PathBuf::from(field("KC"));
    let mode = std::fs::metadata(&kubeconfig).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "private file while the shell runs");
    let dir = kubeconfig.parent().unwrap().to_owned();

    pty.write(b"\n").await.unwrap();
    let end = read_until(&mut stream, |_| false).await;
    assert!(end.status.unwrap().is_success());
    assert!(!kubeconfig.exists(), "removed when the shell exited");

    // No-persistence: nothing of the session is left on disk next to it.
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
        assert!(!text.contains(namespace) && !text.contains(token));
    }
}

#[tokio::test]
async fn dropping_a_running_cluster_terminal_removes_its_file() {
    let src = tempfile::tempdir().unwrap();
    let cluster = ClusterEnv::new(
        ContextName::new("c1"),
        vec![cluster_file(src.path(), "tok-drop")],
    );
    let pty = LocalPty::spawn(sh(r#"echo "$KUBECONFIG"; read _"#).for_cluster(cluster)).unwrap();
    let mut stream = pty.output_stream();
    let run = read_until(&mut stream, |out| out.ends_with('\n')).await;
    let kubeconfig = std::path::PathBuf::from(run.output.trim());
    assert!(kubeconfig.exists());
    drop(pty);
    assert!(!kubeconfig.exists());
}
