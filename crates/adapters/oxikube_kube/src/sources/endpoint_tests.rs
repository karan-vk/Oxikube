//! `cluster_endpoint` (E17-S09): the server address and CA a ServiceAccount's kubeconfig is built
//! from, read from the source kubeconfig. No cluster, no watcher.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::ClusterSourcePort;
use oxikube_testkit::FakeSecretStorePort;
use tempfile::TempDir;

use super::{KubeconfigSources, SourcesConfig};
use crate::kubeconfig::{Env, Platform};

const PEM: &str = "-----BEGIN CERTIFICATE-----\nMIIBfake\n-----END CERTIFICATE-----\n";
const TOKEN: &str = "s3cr3t-token-do-not-leak";

/// A kubeconfig with one context `c` whose cluster entry is `cluster_lines`.
fn config(cluster_lines: &str) -> String {
    format!(
        "apiVersion: v1\nkind: Config\nclusters:\n- name: c\n  cluster:\n{cluster_lines}\
         users:\n- name: u\n  user:\n    token: {TOKEN}\n\
         contexts:\n- name: c\n  context:\n    cluster: c\n    user: u\n"
    )
}

async fn endpoint_of(
    dir: &Path,
    text: &str,
) -> oxikube_domain::OxiResult<oxikube_ports::ClusterEndpoint> {
    let kube = dir.join("home").join(".kube");
    fs::create_dir_all(&kube).unwrap();
    let path = kube.join("config");
    fs::write(&path, text).unwrap();
    let mut config = SourcesConfig::new(Env {
        platform: Platform::host(),
        home: Some(dir.join("home")),
        ..Env::default()
    });
    config.watch = false;
    let sources = KubeconfigSources::new(config, Arc::new(FakeSecretStorePort::new()))
        .expect("no watcher to start");
    let cluster = sources.contexts().await.unwrap().remove(0).cluster;
    sources.cluster_endpoint(&cluster).await
}

#[tokio::test]
async fn inline_ca_data_is_decoded_to_pem() {
    let dir = TempDir::new().unwrap();
    let text = config(&format!(
        "    server: https://api.example:6443\n    certificate-authority-data: {}\n",
        STANDARD.encode(PEM)
    ));
    let endpoint = endpoint_of(dir.path(), &text).await.unwrap();
    assert_eq!(endpoint.server, "https://api.example:6443");
    assert_eq!(endpoint.certificate_authority_pem.as_deref(), Some(PEM));
    assert!(!endpoint.insecure_skip_tls_verify);
    assert!(
        !format!("{endpoint:?}").contains("s3cr3t"),
        "no credential in the endpoint"
    );
}

#[tokio::test]
async fn a_ca_file_is_read_and_returned_inline() {
    let dir = TempDir::new().unwrap();
    let ca = dir.path().join("ca.crt");
    fs::write(&ca, PEM).unwrap();
    let text = config(&format!(
        "    server: https://api.example:6443\n    certificate-authority: {}\n",
        ca.display()
    ));
    let endpoint = endpoint_of(dir.path(), &text).await.unwrap();
    assert_eq!(endpoint.certificate_authority_pem.as_deref(), Some(PEM));
}

#[tokio::test]
async fn a_relative_ca_file_is_found_next_to_the_kubeconfig() {
    let dir = TempDir::new().unwrap();
    let kube = dir.path().join("home").join(".kube");
    fs::create_dir_all(&kube).unwrap();
    fs::write(kube.join("ca.crt"), PEM).unwrap();
    let text = config("    server: https://api.example:6443\n    certificate-authority: ca.crt\n");
    let endpoint = endpoint_of(dir.path(), &text).await.unwrap();
    assert_eq!(endpoint.certificate_authority_pem.as_deref(), Some(PEM));
}

#[tokio::test]
async fn no_ca_and_skipped_verification_are_reported_as_they_are() {
    let dir = TempDir::new().unwrap();
    let text = config(
        "    server: https://api.example:6443\n    insecure-skip-tls-verify: true\n    tls-server-name: kubernetes\n",
    );
    let endpoint = endpoint_of(dir.path(), &text).await.unwrap();
    assert_eq!(endpoint.certificate_authority_pem, None);
    assert!(endpoint.insecure_skip_tls_verify);
    assert_eq!(endpoint.tls_server_name.as_deref(), Some("kubernetes"));
}

#[tokio::test]
async fn a_missing_ca_file_is_a_validation_error_naming_the_file() {
    let dir = TempDir::new().unwrap();
    let text = config(
        "    server: https://api.example:6443\n    certificate-authority: /nonexistent/ca.crt\n",
    );
    let error = endpoint_of(dir.path(), &text).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Validation);
    assert!(
        error.message().contains("/nonexistent/ca.crt"),
        "{}",
        error.message()
    );
}

#[tokio::test]
async fn a_ca_that_is_not_pem_is_refused_without_quoting_it() {
    let dir = TempDir::new().unwrap();
    let text = config(&format!(
        "    server: https://api.example:6443\n    certificate-authority-data: {}\n",
        STANDARD.encode("not a certificate")
    ));
    let error = endpoint_of(dir.path(), &text).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Validation);
    assert!(!error.message().contains("not a certificate"));
}

#[tokio::test]
async fn a_cluster_the_catalog_does_not_have_is_not_found() {
    let dir = TempDir::new().unwrap();
    let kube = dir.path().join("home").join(".kube");
    fs::create_dir_all(&kube).unwrap();
    fs::write(kube.join("config"), config("    server: https://a\n")).unwrap();
    let mut config = SourcesConfig::new(Env {
        platform: Platform::host(),
        home: Some(dir.path().join("home")),
        ..Env::default()
    });
    config.watch = false;
    let sources = KubeconfigSources::new(config, Arc::new(FakeSecretStorePort::new())).unwrap();
    let stranger = ClusterId::new("/elsewhere", &ContextName::new("c"));
    let error = sources.cluster_endpoint(&stranger).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound);
}
