//! No credential reaches `Debug` output, error messages or logs from the source-selection and
//! in-cluster paths (E03-S10): [`Env`], [`Selection`], a [`LoadedKubeconfig`] selected through
//! `KUBECONFIG`, a `token-file` credential, an unparsable kubeconfig that holds a token, and the
//! synthetic in-cluster context through the pool. Companion to `debug_redaction.rs`.

mod support;

use std::path::{Path, PathBuf};

use kube::config::Kubeconfig;
use oxikube_domain::ids::ContextName;
use oxikube_kube::kubeconfig::{
    Env, IN_CLUSTER_CONTEXT, LoadedKubeconfig, Platform, Strictness,
    load_kubeconfig_for_env_blocking, select_sources,
};
use oxikube_kube::{ClientPool, ContextDefinition, PoolConfig};
use support::{assert_error_clean, assert_no_secrets, captured, init_tracing};

// Fake secrets. Nothing here is a real credential.
const TOKEN: &str = "src-token-BBBB-9876543210";
const TOKEN_FILE_SECRET: &str = "src-token-file-contents-CCCC";
const PARSE_SECRET: &str = "src-unparsable-secret-DDDD";
const ALL: &[&str] = &[TOKEN, TOKEN_FILE_SECRET, PARSE_SECRET];

#[track_caller]
fn assert_debug_clean(what: &str, value: &dyn std::fmt::Debug) {
    support::assert_debug_clean(what, value, ALL);
}

/// A kubeconfig with an inline token user and a `token-file` user, written under `dir`.
fn write_kubeconfig(dir: &Path) -> PathBuf {
    let token_file = dir.join("token");
    std::fs::write(&token_file, TOKEN_FILE_SECRET).unwrap();
    let yaml = format!(
        r#"
apiVersion: v1
kind: Config
current-context: token
clusters:
- name: plain
  cluster: {{server: "https://10.2.3.4:6443", insecure-skip-tls-verify: true}}
users:
- name: token-user
  user: {{token: {TOKEN}}}
- name: file-user
  user: {{token-file: "{}"}}
contexts:
- name: token
  context: {{cluster: plain, user: token-user}}
- name: token-file
  context: {{cluster: plain, user: file-user}}
"#,
        token_file.display()
    );
    let path = dir.join("config");
    std::fs::write(&path, yaml).unwrap();
    path
}

/// An environment that selects `kubeconfig` through `KUBECONFIG` and is not a pod.
fn env_with_kubeconfig(kubeconfig: &Path) -> Env {
    Env {
        platform: Platform::host(),
        kubeconfig: Some(kubeconfig.as_os_str().to_owned()),
        ..Env::default()
    }
}

/// An environment that looks like a pod and has no kubeconfig source at all.
fn pod_env() -> Env {
    Env {
        platform: Platform::host(),
        kubernetes_service_host: Some("10.96.0.1".into()),
        kubernetes_service_port: Some("443".into()),
        service_account_mounted: true,
        service_account_namespace: Some("tools".into()),
        ..Env::default()
    }
}

fn definitions(loaded: &LoadedKubeconfig) -> Vec<ContextDefinition> {
    loaded
        .context_names()
        .filter_map(|name| ContextDefinition::from_kubeconfig(&loaded.merged, &name))
        .collect()
}

#[test]
fn env_selection_and_env_selected_kubeconfig_debug_are_clean() {
    let dir = tempfile::tempdir().unwrap();
    let env = env_with_kubeconfig(&write_kubeconfig(dir.path()));
    assert_debug_clean("Env", &env);
    assert_debug_clean("Selection", &select_sources(&[], &env));

    let loaded = load_kubeconfig_for_env_blocking(&[], &env, Strictness::RequireUsable).unwrap();
    assert_eq!(loaded.context_names().count(), 2, "{loaded:?}");
    assert_debug_clean("LoadedKubeconfig (KUBECONFIG tier)", &loaded);
    for diagnostic in &loaded.diagnostics {
        assert_no_secrets("diagnostic", &diagnostic.to_string(), ALL);
    }
    let defs = definitions(&loaded);
    assert_eq!(defs.len(), 2);
    for definition in &defs {
        assert_debug_clean("ContextDefinition (KUBECONFIG tier)", definition);
    }
}

#[tokio::test]
async fn token_file_credential_never_reaches_debug_or_logs() {
    init_tracing();
    let dir = tempfile::tempdir().unwrap();
    let env = env_with_kubeconfig(&write_kubeconfig(dir.path()));
    let loaded = load_kubeconfig_for_env_blocking(&[], &env, Strictness::RequireUsable).unwrap();
    let pool = ClientPool::from_loaded(&loaded, PoolConfig::default());
    for name in ["token", "token-file"] {
        pool.get(&ContextName::from(name))
            .await
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
    }
    let shown = format!("{pool:?}");
    assert!(shown.contains("built: true"), "{shown}");
    assert_debug_clean("ClientPool (token file)", &pool);
    tracing::trace!(?pool, ?loaded, "pool ready");
    let out = captured();
    assert!(out.contains("pool ready"), "capture is live:\n{out}");
    assert_no_secrets("trace output", &out, ALL);
}

#[test]
fn an_unparsable_kubeconfig_holding_a_token_reports_no_contents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("broken");
    // `contexts` must be a list; the parser's message quotes the offending string.
    let yaml = format!("apiVersion: v1\nkind: Config\ncontexts: \"token {PARSE_SECRET}\"\n");
    std::fs::write(&path, &yaml).unwrap();
    let raw = Kubeconfig::from_yaml(&yaml).unwrap_err().to_string();
    assert!(
        raw.contains(PARSE_SECRET),
        "the parser quotes the value, so the check below is not vacuous: {raw}"
    );

    let env = env_with_kubeconfig(&path);
    let tolerant = load_kubeconfig_for_env_blocking(&[], &env, Strictness::Tolerant).unwrap();
    assert_debug_clean("LoadedKubeconfig (unparsable)", &tolerant);
    for diagnostic in &tolerant.diagnostics {
        assert_no_secrets("diagnostic", &diagnostic.to_string(), ALL);
    }
    let err = load_kubeconfig_for_env_blocking(&[], &env, Strictness::RequireUsable).unwrap_err();
    assert!(err.message().contains("not a valid kubeconfig"), "{err:?}");
    assert_error_clean("strict load (unparsable)", &err, ALL);
}

#[tokio::test]
async fn in_cluster_context_debug_and_build_errors_are_clean() {
    init_tracing();
    let env = pod_env();
    assert_debug_clean("Env (pod)", &env);
    let loaded = load_kubeconfig_for_env_blocking(&[], &env, Strictness::RequireUsable).unwrap();
    let context = ContextName::from(IN_CLUSTER_CONTEXT);
    assert!(loaded.is_in_cluster(&context), "{loaded:?}");
    assert_debug_clean("LoadedKubeconfig (in-cluster)", &loaded);

    let definition = ContextDefinition::from_kubeconfig(&loaded.merged, &context)
        .unwrap()
        .with_in_cluster(true);
    let shown = format!("{definition:?}");
    assert!(shown.contains("10.96.0.1"), "host is still shown: {shown}");
    assert_debug_clean("ContextDefinition (in-cluster)", &definition);

    // Outside a pod the mounted CA and token are absent, so the build fails; inside one it
    // succeeds. Either way nothing secret may surface.
    let pool = ClientPool::from_loaded(&loaded, PoolConfig::default());
    if let Err(err) = pool.get(&context).await {
        assert_error_clean("in-cluster build", &err, ALL);
        tracing::trace!(?err, "in-cluster build failed");
    }
    assert_debug_clean("ClientPool (in-cluster)", &pool);
    assert_no_secrets("trace output", &captured(), ALL);
}
