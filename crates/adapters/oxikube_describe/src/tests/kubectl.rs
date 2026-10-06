//! The `kubectl` backend against a stub script that prints its arguments (unix only).

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{ContextName, Gvk, ResourceRef};
use oxikube_ports::{DescribePort, DescribeSource};

use super::{cluster, discovery, pod_ref};
use crate::{Backend, DescribeConfig, DescribePreference, KubectlDescribe, KubectlTarget};

fn script(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("kubectl-stub");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn kubectl(binary: Option<PathBuf>, kubeconfig: Option<PathBuf>) -> KubectlDescribe {
    KubectlDescribe::new(
        discovery(),
        DescribePreference::new(DescribeConfig {
            backend: Backend::Kubectl,
            kubectl_path: binary,
        }),
        KubectlTarget {
            context: ContextName::new("kind-test"),
            kubeconfig,
        },
    )
}

#[tokio::test]
async fn the_command_line_names_the_context_the_kind_and_the_object() {
    let dir = tempfile::tempdir().unwrap();
    let binary = script(dir.path(), r#"echo "args: $@""#);
    let describer = kubectl(Some(binary), Some("/work/prod.yaml".into()));
    let output = describer.describe(&pod_ref()).await.unwrap();
    assert_eq!(output.source, DescribeSource::KubectlFallback);
    assert_eq!(
        output.text.trim(),
        "args: --context kind-test --kubeconfig /work/prod.yaml describe pods web-running --namespace demo"
    );
}

#[tokio::test]
async fn a_grouped_kind_is_plural_dot_group_and_cluster_scoped_has_no_namespace() {
    let dir = tempfile::tempdir().unwrap();
    let binary = script(dir.path(), r#"echo "$@""#);
    let describer = kubectl(Some(binary), None);
    let deployment = ResourceRef::namespaced(
        cluster(),
        Gvk::new("apps", "v1", "Deployment"),
        "demo",
        "web",
    );
    let output = describer.describe(&deployment).await.unwrap();
    assert!(
        output.text.contains("describe deployments.apps web"),
        "{}",
        output.text
    );
    assert!(!output.text.contains("--kubeconfig"), "{}", output.text);
}

#[tokio::test]
async fn a_failing_kubectl_is_classified_from_its_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let binary = script(
        dir.path(),
        r#"echo 'Error from server (NotFound): pods "web-running" not found' >&2; exit 1"#,
    );
    let error = kubectl(Some(binary), None)
        .describe(&pod_ref())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound, "{error}");
}

#[tokio::test]
async fn a_missing_binary_is_unsupported_with_a_hint() {
    let describer = kubectl(Some("/nonexistent/kubectl".into()), None);
    let error = describer.describe(&pod_ref()).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported, "{error}");
    assert!(error.message().contains("describe.kubectl_path"), "{error}");
}

#[tokio::test]
async fn the_child_is_killed_when_the_call_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    // `exec` so the pid written is the one that sleeps.
    let binary = script(
        dir.path(),
        &format!("echo $$ > {}; exec sleep 30", pid_file.display()),
    );
    let describer = kubectl(Some(binary), None);
    let target = pod_ref();
    let call = describer.describe(&target);
    let timed_out = tokio::time::timeout(std::time::Duration::from_millis(500), call).await;
    assert!(timed_out.is_err(), "the stub sleeps");
    let pid = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .to_owned();
    // The future was dropped: `kill_on_drop` ends the child (allow the signal a moment).
    let alive = || {
        std::process::Command::new("kill")
            .args(["-0", &pid])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    };
    for _ in 0..50 {
        if !alive() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("the kubectl child {pid} outlived the dropped call");
}
