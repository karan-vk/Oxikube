//! Rendering the template: the pod and the command from the settings.

use std::time::Duration;

use oxikube_domain::ErrorKind;
use serde_json::json;

use super::*;
use crate::cluster_prefs::{ClusterPrefs, NodeShellPrefs};

fn spec() -> NodeShellSpec {
    NodeShellSpec::new("worker-1")
}

#[test]
fn the_default_pod_is_privileged_in_the_host_namespaces_and_pinned_to_the_node() {
    let pod = node_shell_manifest(&spec()).expect("manifest");
    assert_eq!(pod["metadata"]["generateName"], "oxikube-node-shell-");
    assert_eq!(pod["metadata"]["namespace"], "kube-system");
    assert_eq!(pod["metadata"]["labels"][NODE_SHELL_LABEL], "true");
    assert_eq!(
        pod["metadata"]["labels"]["app.kubernetes.io/managed-by"],
        "oxikube"
    );
    assert_eq!(pod["metadata"]["annotations"][NODE_ANNOTATION], "worker-1");
    let spec = &pod["spec"];
    assert_eq!(spec["nodeName"], "worker-1");
    assert_eq!(spec["hostPID"], true);
    assert_eq!(spec["hostNetwork"], true);
    assert_eq!(spec["hostIPC"], true);
    assert_eq!(spec["restartPolicy"], "Never");
    assert_eq!(spec["activeDeadlineSeconds"], 8 * 60 * 60);
    assert_eq!(spec["tolerations"], json!([{"operator": "Exists"}]));
    assert!(spec.get("imagePullSecrets").is_none());
    let container = &spec["containers"][0];
    assert_eq!(container["name"], CONTAINER_NAME);
    assert_eq!(container["image"], "busybox:1.37");
    assert_eq!(container["securityContext"]["privileged"], true);
    assert!(container.get("imagePullPolicy").is_none());
    assert_eq!(container["command"], json!(["sleep", "28800"]));
}

#[test]
fn the_settings_fill_the_template() {
    let prefs = ClusterPrefs {
        node_shell_image: Some("registry.local/tools:1".into()),
        node_shell_pull_secret: Some("regcred".into()),
        node_shell: NodeShellPrefs {
            namespace: Some("debug".into()),
            command: vec!["/bin/zsh".into(), "-l".into()],
            nsenter_args: vec!["-t".into(), "1".into(), "-m".into(), "-n".into()],
            tolerations: Some(vec![
                NodeShellToleration {
                    key: Some("dedicated".into()),
                    operator: Some("Equal".into()),
                    value: Some("gpu".into()),
                    effect: Some("NoSchedule".into()),
                    toleration_seconds: None,
                },
                NodeShellToleration {
                    operator: Some("Exists".into()),
                    effect: Some("NoExecute".into()),
                    toleration_seconds: Some(30),
                    ..NodeShellToleration::default()
                },
            ]),
            labels: [("team".to_owned(), "infra".to_owned())].into(),
            image_pull_policy: Some("Always".into()),
            max_lifetime_seconds: Some(600),
        },
        ..ClusterPrefs::default()
    };
    let spec = NodeShellSpec::for_node("worker-1", &prefs);
    assert_eq!(spec.max_lifetime, Duration::from_secs(600));
    let pod = node_shell_manifest(&spec).expect("manifest");
    assert_eq!(pod["metadata"]["namespace"], "debug");
    assert_eq!(pod["metadata"]["labels"]["team"], "infra");
    assert_eq!(pod["metadata"]["labels"][NODE_SHELL_LABEL], "true");
    let pod_spec = &pod["spec"];
    assert_eq!(pod_spec["activeDeadlineSeconds"], 600);
    assert_eq!(pod_spec["imagePullSecrets"][0]["name"], "regcred");
    assert_eq!(
        pod_spec["tolerations"],
        json!([
            {"key": "dedicated", "operator": "Equal", "value": "gpu", "effect": "NoSchedule"},
            {"operator": "Exists", "effect": "NoExecute", "tolerationSeconds": 30},
        ])
    );
    let container = &pod_spec["containers"][0];
    assert_eq!(container["image"], "registry.local/tools:1");
    assert_eq!(container["imagePullPolicy"], "Always");
    assert_eq!(container["command"], json!(["sleep", "600"]));
    assert_eq!(
        node_shell_command(&spec),
        ["nsenter", "-t", "1", "-m", "-n", "--", "/bin/zsh", "-l"]
    );
}

#[test]
fn unset_settings_keep_the_defaults_and_an_empty_toleration_list_is_honoured() {
    let spec = NodeShellSpec::for_node("n", &ClusterPrefs::default());
    assert_eq!(spec, NodeShellSpec::new("n"));
    let prefs = ClusterPrefs {
        node_shell: NodeShellPrefs {
            tolerations: Some(Vec::new()),
            ..NodeShellPrefs::default()
        },
        ..ClusterPrefs::default()
    };
    let pod = node_shell_manifest(&NodeShellSpec::for_node("n", &prefs)).expect("manifest");
    assert_eq!(pod["spec"]["tolerations"], json!([]), "tolerate nothing");
}

#[test]
fn the_user_cannot_replace_the_labels_the_sweep_selects_on() {
    let mut spec = spec();
    spec.labels.insert(NODE_SHELL_LABEL.into(), "false".into());
    spec.labels
        .insert("app.kubernetes.io/managed-by".into(), "someone".into());
    let pod = node_shell_manifest(&spec).expect("manifest");
    assert_eq!(pod["metadata"]["labels"][NODE_SHELL_LABEL], "true");
    assert_eq!(
        pod["metadata"]["labels"]["app.kubernetes.io/managed-by"],
        "oxikube"
    );
}

#[test]
fn the_default_command_enters_every_namespace_and_prefers_bash() {
    let command = node_shell_command(&spec());
    assert_eq!(
        &command[..9],
        ["nsenter", "-t", "1", "-m", "-u", "-i", "-n", "-p", "--"]
    );
    assert_eq!(&command[9..11], ["sh", "-c"]);
    assert!(command[11].contains("exec bash -l") && command[11].contains("exec sh -l"));
}

#[test]
fn a_bad_template_is_a_validation_error() {
    let bad = |spec: NodeShellSpec| {
        let err = node_shell_manifest(&spec).expect_err("refused");
        assert_eq!(err.kind(), ErrorKind::Validation, "{err}");
    };
    bad(NodeShellSpec {
        image: "  ".into(),
        ..spec()
    });
    bad(NodeShellSpec {
        namespace: "a/b".into(),
        ..spec()
    });
    bad(NodeShellSpec::new("a/b"));
    bad(NodeShellSpec::new(""));
    bad(NodeShellSpec {
        image_pull_policy: Some("Sometimes".into()),
        ..spec()
    });
    bad(NodeShellSpec {
        nsenter_args: vec!["-t".into(), " ".into()],
        ..spec()
    });
}
