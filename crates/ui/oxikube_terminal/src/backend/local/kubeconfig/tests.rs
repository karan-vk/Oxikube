use std::path::Path;

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::ContextName;
use oxikube_ports::cluster_source::{ClusterSource, SourceId, SourceKind};
use serde_json::Value;

use super::*;

const PROD: &str = r#"
apiVersion: v1
kind: Config
current-context: dev
contexts:
  - name: prod
    context: { cluster: prod-cluster, user: prod-user, namespace: payments }
  - name: dev
    context: { cluster: dev-cluster, user: dev-user }
clusters:
  - name: prod-cluster
    cluster: { server: "https://prod.example", certificate-authority: certs/ca.pem }
  - name: dev-cluster
    cluster: { server: "https://dev.example" }
users:
  - name: prod-user
    user:
      token: s3cr3t-token-value
      client-key: keys/prod.key
      exec: { command: ./bin/auth, apiVersion: client.authentication.k8s.io/v1 }
  - name: dev-user
    user: { token: other-token }
"#;

/// A second file: defines a user the first lacks, and a duplicate of `dev-cluster` that must lose.
const EXTRA: &str = r#"
apiVersion: v1
kind: Config
contexts:
  - name: staging
    context: { cluster: stage-cluster, user: stage-user }
clusters:
  - name: stage-cluster
    cluster: { server: "https://stage.example" }
  - name: dev-cluster
    cluster: { server: "https://shadowed.example" }
users:
  - name: stage-user
    user: { tokenFile: tokens/stage }
"#;

fn write(dir: &Path, name: &str, text: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

fn env(context: &str, files: Vec<std::path::PathBuf>) -> ClusterEnv {
    ClusterEnv::new(ContextName::new(context), files)
}

fn merged(env: &ClusterEnv) -> Value {
    serde_json::from_str(&merged_kubeconfig(env).unwrap().text).unwrap()
}

#[test]
fn the_merged_file_holds_only_the_selected_context() {
    let dir = tempfile::tempdir().unwrap();
    let files = vec![write(dir.path(), "a.yaml", PROD)];
    let doc = merged(&env("prod", files));
    assert_eq!(doc["current-context"], "prod");
    assert_eq!(doc["contexts"].as_array().unwrap().len(), 1);
    assert_eq!(doc["contexts"][0]["context"]["cluster"], "prod-cluster");
    assert_eq!(doc["contexts"][0]["context"]["namespace"], "payments");
    assert_eq!(doc["clusters"].as_array().unwrap().len(), 1);
    assert_eq!(doc["users"].as_array().unwrap().len(), 1);
    let text = doc.to_string();
    assert!(!text.contains("other-token") && !text.contains("dev.example"));
}

#[test]
fn the_namespace_is_the_override_then_the_contexts_then_default() {
    let dir = tempfile::tempdir().unwrap();
    let files = vec![write(dir.path(), "a.yaml", PROD)];
    let ns = |env: ClusterEnv| merged_kubeconfig(&env).unwrap().namespace;
    assert_eq!(ns(env("prod", files.clone())), "payments");
    assert_eq!(
        ns(env("prod", files.clone()).in_namespace("kube-system")),
        "kube-system"
    );
    assert_eq!(ns(env("dev", files)), "default");
}

#[test]
fn relative_paths_resolve_against_the_file_that_defines_them() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("conf");
    std::fs::create_dir(&sub).unwrap();
    let files = vec![write(&sub, "a.yaml", PROD)];
    let doc = merged(&env("prod", files));
    let ca = doc["clusters"][0]["cluster"]["certificate-authority"]
        .as_str()
        .unwrap();
    assert_eq!(Path::new(ca), sub.join("certs/ca.pem"));
    let user = &doc["users"][0]["user"];
    assert_eq!(
        Path::new(user["client-key"].as_str().unwrap()),
        sub.join("keys/prod.key")
    );
    assert_eq!(
        Path::new(user["exec"]["command"].as_str().unwrap()),
        sub.join("./bin/auth")
    );
}

#[test]
fn the_first_file_defining_a_name_wins_and_entries_come_from_any_file() {
    let dir = tempfile::tempdir().unwrap();
    let files = vec![
        write(dir.path(), "a.yaml", PROD),
        write(dir.path(), "b.yaml", EXTRA),
    ];
    let dev = merged(&env("dev", files.clone()));
    assert_eq!(
        dev["clusters"][0]["cluster"]["server"],
        "https://dev.example"
    );
    let staging = merged(&env("staging", files));
    assert_eq!(
        staging["clusters"][0]["cluster"]["server"],
        "https://stage.example"
    );
    assert_eq!(
        Path::new(staging["users"][0]["user"]["tokenFile"].as_str().unwrap()),
        dir.path().join("tokens/stage")
    );
}

#[test]
fn an_unknown_context_is_not_found_and_the_message_quotes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let files = vec![write(dir.path(), "a.yaml", PROD)];
    let error = merged_kubeconfig(&env("nope", files)).err().unwrap();
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(!error.to_string().contains("s3cr3t"));
}

#[test]
fn a_broken_or_missing_file_is_skipped_not_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let files = vec![
        dir.path().join("missing.yaml"),
        write(dir.path(), "junk.yaml", "{{{ not yaml"),
        write(dir.path(), "a.yaml", PROD),
    ];
    assert_eq!(merged(&env("dev", files))["current-context"], "dev");
}

#[cfg(unix)]
#[test]
fn the_temp_file_is_private_and_removed_on_drop() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let files = vec![write(dir.path(), "a.yaml", PROD)];
    let prepared = env("prod", files).in_namespace("web").prepare().unwrap();
    let path = prepared.file.path().to_owned();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(path.parent().unwrap()), 0o700);
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("prod.example")
    );

    let vars: std::collections::HashMap<_, _> = prepared.vars.iter().cloned().collect();
    assert_eq!(vars["KUBECONFIG"], path.to_string_lossy());
    assert_eq!(vars["KUBE_CONTEXT"], "prod");
    assert_eq!(vars["OXIKUBE_NAMESPACE"], "web");
    assert!(!vars.contains_key("PATH"));
    assert!(
        vars.values().all(|v| !v.contains("s3cr3t")),
        "no credential in the environment"
    );

    drop(prepared);
    assert!(!path.exists());
}

#[test]
fn files_of_sources_expands_files_and_directories() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "a.yaml", PROD);
    let conf = dir.path().join("conf.d");
    std::fs::create_dir(&conf).unwrap();
    let c2 = write(&conf, "2.yaml", EXTRA);
    let c1 = write(&conf, "1.yaml", PROD);
    let source = |kind, path: Option<std::path::PathBuf>| ClusterSource {
        id: SourceId("s".into()),
        kind,
        label: String::new(),
        path,
    };
    let sources = [
        source(SourceKind::KubeconfigFile, Some(a.clone())),
        source(SourceKind::KubeconfigDir, Some(conf)),
        source(SourceKind::InCluster, None),
    ];
    assert_eq!(files_of_sources(&sources), vec![a, c1, c2]);
}
