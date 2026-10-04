//! The in-cluster fix-ups reach the pool's config build by provenance (E03-S10).
//!
//! A real client for the synthetic context cannot be built off a pod (its CA file is mounted
//! only there), so the pool-level tests record what the factory is handed, and the config-level
//! tests flag an ordinary definition.

use std::sync::Arc;

use kube::Client;
use kube::config::Kubeconfig;
use oxikube_domain::ids::ContextName;
use oxikube_domain::{ErrorKind, OxiError};
use parking_lot::Mutex;
use tempfile::TempDir;

use super::*;
use crate::kubeconfig::{
    Env, IN_CLUSTER_CONTEXT, LoadedKubeconfig, Strictness, load_kubeconfig_for_env_blocking,
    load_kubeconfig_from_paths_blocking,
};

const CA_FILE: &str = "/var/run/secrets/kubernetes.io/serviceaccount/ca.crt";
const PROXY: &str = "http://proxy.example:3128";

fn yaml_with_proxy() -> Kubeconfig {
    Kubeconfig::from_yaml(&format!(
        "clusters:\n- name: c\n  cluster:\n    server: https://127.0.0.1:1\n    insecure-skip-tls-verify: true\n    proxy-url: {PROXY}\n\
         users:\n- name: u\n  user:\n    token: t\n\
         contexts:\n- name: x\n  context: {{cluster: c, user: u}}\n"
    ))
    .unwrap()
}

fn definition(in_cluster: bool) -> ContextDefinition {
    ContextDefinition::from_kubeconfig(&yaml_with_proxy(), &ContextName::from("x"))
        .unwrap()
        .with_in_cluster(in_cluster)
}

#[test]
fn build_config_applies_the_in_cluster_fixups_only_when_flagged() {
    let env = ProxyEnv::with_https_proxy(Some("http://env-proxy.example:1".into()));

    let flagged = build_config(&definition(true), &PoolConfig::default(), &env).unwrap();
    assert_eq!(
        flagged.root_cert_file.as_deref(),
        Some(std::path::Path::new(CA_FILE))
    );
    assert!(
        flagged.proxy_url.is_none(),
        "no proxy, whatever the cluster or env says"
    );

    let plain = build_config(&definition(false), &PoolConfig::default(), &env).unwrap();
    assert!(plain.root_cert_file.is_none());
    assert_eq!(
        plain.proxy_url.map(|u| u.to_string()).as_deref(),
        Some("http://proxy.example:3128/")
    );
}

#[test]
fn provenance_is_part_of_the_connection_identity() {
    assert!(definition(true).same_connection(&definition(true)));
    assert!(!definition(true).same_connection(&definition(false)));
}

/// Records which contexts the pool asked for and whether they were flagged in-cluster, then
/// fails the build (nothing here needs a client).
#[derive(Default)]
struct Recorder(Mutex<Vec<(String, bool)>>);

impl ClientFactory for Recorder {
    fn build(&self, definition: &ContextDefinition, _: &PoolConfig) -> Result<Client, OxiError> {
        self.0
            .lock()
            .push((definition.context().to_string(), definition.is_in_cluster()));
        Err(OxiError::internal("recorded"))
    }
}

fn pod_env() -> Env {
    Env {
        kubernetes_service_host: Some("10.0.0.1".into()),
        kubernetes_service_port: Some("443".into()),
        service_account_mounted: true,
        ..Env::default()
    }
}

fn pool_over(loaded: &LoadedKubeconfig, recorder: &Arc<Recorder>) -> ClientPool {
    let pool = ClientPool::with_parts(
        Kubeconfig::default(),
        PoolConfig::default(),
        recorder.clone(),
        Arc::new(SystemClock),
    );
    pool.replace_loaded(loaded);
    pool
}

#[tokio::test]
async fn synthetic_in_cluster_context_is_flagged_for_the_factory() {
    let loaded = load_kubeconfig_for_env_blocking(&[], &pod_env(), Strictness::Tolerant).unwrap();
    assert!(loaded.is_in_cluster(&IN_CLUSTER_CONTEXT.into()));
    let recorder = Arc::new(Recorder::default());
    let pool = pool_over(&loaded, &recorder);

    let err = pool
        .get(&IN_CLUSTER_CONTEXT.into())
        .await
        .err()
        .expect("no client off a pod");

    assert_eq!(err.kind(), ErrorKind::Internal);
    assert_eq!(*recorder.0.lock(), [(IN_CLUSTER_CONTEXT.to_owned(), true)]);
}

#[tokio::test]
async fn file_context_named_in_cluster_is_not_flagged() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("argo");
    std::fs::write(
        &file,
        "clusters:\n- name: in-cluster\n  cluster:\n    server: https://kubernetes.default.svc\n\
         contexts:\n- name: in-cluster\n  context:\n    cluster: in-cluster\n",
    )
    .unwrap();
    let loaded = load_kubeconfig_from_paths_blocking(&[file], Strictness::Tolerant).unwrap();
    let recorder = Arc::new(Recorder::default());
    let pool = pool_over(&loaded, &recorder);

    let _ = pool.get(&IN_CLUSTER_CONTEXT.into()).await;

    assert_eq!(*recorder.0.lock(), [(IN_CLUSTER_CONTEXT.to_owned(), false)]);
}

#[tokio::test]
async fn from_loaded_records_provenance_too() {
    let loaded = load_kubeconfig_for_env_blocking(&[], &pod_env(), Strictness::Tolerant).unwrap();
    let pool = ClientPool::from_loaded(&loaded, PoolConfig::default());
    // The real factory fails off a pod (the CA file is not there): the point is only that the
    // flagged definition reached it, which shows as a CA read failure, not "not in the kubeconfig".
    let err = pool
        .get(&IN_CLUSTER_CONTEXT.into())
        .await
        .err()
        .expect("no client off a pod");
    assert_ne!(err.kind(), ErrorKind::NotFound, "{err}");
}
