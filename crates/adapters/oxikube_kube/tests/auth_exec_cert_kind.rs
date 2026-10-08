//! Kind integration: an exec plugin that returns the kind admin's client certificate is run
//! a bounded number of times per client build and the identity it hands out is the one the
//! API server sees (E03-F550). Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips
//! cleanly otherwise, or when the kind user authenticates some way other than a certificate.
#![cfg(all(feature = "integration", unix))]

mod common;

use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use kube::config::{AuthInfo, Kubeconfig};
use secrecy::ExposeSecret as _;

use common::whoami;

/// The kind user's certificate and key as PEM, whichever way the kubeconfig stores them.
fn kind_identity(kubeconfig: &Kubeconfig) -> Option<(String, String)> {
    let user: &AuthInfo = kubeconfig.auth_infos.first()?.auth_info.as_ref()?;
    let read = |data: Option<&str>, file: &Option<String>| -> Option<String> {
        match (data, file) {
            (Some(data), _) => String::from_utf8(STANDARD.decode(data).ok()?).ok(),
            (None, Some(file)) => std::fs::read_to_string(file).ok(),
            (None, None) => None,
        }
    };
    let cert = read(
        user.client_certificate_data.as_deref(),
        &user.client_certificate,
    )?;
    let key = read(
        user.client_key_data.as_ref().map(|k| k.expose_secret()),
        &user.client_key,
    )?;
    Some((cert, key))
}

/// A kubeconfig for the kind cluster whose only credential is a plugin that prints `cert` and
/// `key` after appending a line to `<dir>/runs`.
fn exec_kubeconfig(kind: &common::Kind, dir: &Path, cert: &str, key: &str) -> Kubeconfig {
    let credential = serde_json::json!({
        "apiVersion": "client.authentication.k8s.io/v1",
        "kind": "ExecCredential",
        "status": {
            "clientCertificateData": cert,
            "clientKeyData": key,
            "expirationTimestamp": "2099-01-01T00:00:00Z",
        }
    });
    std::fs::write(dir.join("credential.json"), credential.to_string()).unwrap();
    let script = dir.join("plugin.sh");
    std::fs::write(
        &script,
        format!(
            "echo run >> {}\ncat {}\n",
            dir.join("runs").display(),
            dir.join("credential.json").display()
        ),
    )
    .unwrap();
    let mut kubeconfig = kind.kubeconfig.clone();
    let user = kubeconfig.auth_infos[0]
        .auth_info
        .as_mut()
        .expect("kind user");
    *user = AuthInfo {
        exec: Some(kube::config::ExecConfig {
            api_version: Some("client.authentication.k8s.io/v1".into()),
            command: Some("sh".into()),
            args: Some(vec![script.to_string_lossy().into_owned()]),
            interactive_mode: Some(kube::config::ExecInteractiveMode::Never),
            ..Default::default()
        }),
        ..AuthInfo::default()
    };
    kubeconfig
}

#[tokio::test]
async fn the_certificate_a_plugin_returns_authenticates_and_the_plugin_is_not_run_per_kube_call() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let Some((cert, key)) = kind_identity(&kind.kubeconfig) else {
        eprintln!("skipped: the kind user does not authenticate with a client certificate");
        return;
    };
    let expected = whoami(kind.admin_client().await.as_ref())
        .await
        .expect("whoami");

    let dir = tempfile::tempdir().unwrap();
    let kubeconfig = exec_kubeconfig(&kind, dir.path(), &cert, &key);
    let client = kind
        .pool(kubeconfig)
        .get(&kind.context)
        .await
        .expect("build through the exec certificate");
    let runs = || {
        std::fs::read_to_string(dir.path().join("runs"))
            .unwrap()
            .lines()
            .count()
    };
    assert_eq!(runs(), 2, "the probe and one run of ours; kube runs none");

    // The TLS handshake presented the plugin's certificate: the server knows who we are.
    assert_eq!(whoami(client.as_ref()).await.expect("whoami"), expected);
    client.apiserver_version().await.expect("version");
    assert_eq!(runs(), 2, "requests never run the plugin");
    assert!(client.valid_until().is_some());
}
