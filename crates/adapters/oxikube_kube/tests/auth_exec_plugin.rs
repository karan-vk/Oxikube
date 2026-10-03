//! Exec credential plugin tests with tiny `sh` scripts as the plugin (unix only).
//!
//! Building a kube `Client` runs the plugin synchronously, which is why production code
//! must do it on the blocking pool. These tests need no cluster: the plugin runs before any
//! connection is made.
#![cfg(unix)]

use std::path::Path;

use kube::config::{ExecConfig, ExecInteractiveMode};
use kube::{Client, Config};
use oxikube_domain::ErrorKind;
use oxikube_kube::auth::{ExecInteractivePolicy, classify};

const OK_CREDENTIAL: &str = r#"printf '{"apiVersion":"client.authentication.k8s.io/v1","kind":"ExecCredential","status":{"token":"FAKE-PLUGIN-TOKEN","expirationTimestamp":"2099-01-01T00:00:00Z"}}'"#;

fn config_with_plugin(dir: &Path, script: &str, mode: Option<ExecInteractiveMode>) -> Config {
    let path = dir.join("plugin.sh");
    std::fs::write(&path, script).unwrap();
    let mut config = Config::new("http://127.0.0.1:1".parse().unwrap());
    // `sh script` rather than executing the file: avoids ETXTBSY races between tests.
    config.auth_info.exec = Some(ExecConfig {
        api_version: Some("client.authentication.k8s.io/v1".into()),
        command: Some("sh".into()),
        args: Some(vec![path.to_string_lossy().into_owned()]),
        interactive_mode: mode,
        ..Default::default()
    });
    config
}

/// Builds the client (runs the plugin) and, when that works, makes one request, so a
/// failure from either stage comes back as a `kube::Error`.
async fn connect(config: Config) -> Result<(), kube::Error> {
    let client = tokio::task::spawn_blocking(move || Client::try_from(config))
        .await
        .unwrap()?;
    client.apiserver_version().await.map(|_| ())
}

#[tokio::test]
async fn successful_plugin_reaches_the_network_and_refusal_is_network() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_with_plugin(dir.path(), OK_CREDENTIAL, Some(ExecInteractiveMode::Never));
    let err = connect(config)
        .await
        .expect_err("nothing listens on port 1");
    assert_eq!(classify(&err).kind(), ErrorKind::Network, "{err}");
}

#[tokio::test]
async fn non_zero_exit_is_auth_and_does_not_leak_the_command_line() {
    let dir = tempfile::tempdir().unwrap();
    let script = "echo 'error: credentials service unavailable' >&2\necho 'Authorization: Bearer FAKE.LEAK.TOKEN-0123456789' >&2\nexit 1\n";
    let config = config_with_plugin(dir.path(), script, Some(ExecInteractiveMode::Never));
    let err = connect(config).await.unwrap_err();
    let oxi = classify(&err);
    assert_eq!(oxi.kind(), ErrorKind::Auth, "{err}");
    assert!(oxi.is_retryable());
    assert!(oxi.message().contains("credentials service unavailable"));
    assert!(!format!("{oxi:?}").contains("FAKE.LEAK.TOKEN"));
    assert!(
        !format!("{oxi:?}").contains("plugin.sh"),
        "command line must not appear"
    );
}

#[tokio::test]
async fn missing_plugin_binary_is_a_permanent_auth_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_with_plugin(dir.path(), "", None);
    config.auth_info.exec.as_mut().unwrap().command =
        Some("/nonexistent/oxikube-fake-plugin".into());
    let err = connect(config).await.unwrap_err();
    let oxi = classify(&err);
    assert_eq!(oxi.kind(), ErrorKind::Auth, "{err}");
    assert!(!oxi.is_retryable());
}

#[tokio::test]
async fn plugin_that_prompts_fails_fast_under_never_instead_of_hanging() {
    let dir = tempfile::tempdir().unwrap();
    let script = "echo 'Enter MFA code:' >&2\nread code || exit 2\nexit 2\n";
    let mut config = config_with_plugin(dir.path(), script, Some(ExecInteractiveMode::IfAvailable));
    ExecInteractivePolicy::Never
        .apply_to_config(&mut config)
        .unwrap();
    let err = tokio::time::timeout(std::time::Duration::from_secs(20), connect(config))
        .await
        .expect("plugin must not hang waiting for input")
        .unwrap_err();
    let oxi = classify(&err);
    assert_eq!(oxi.kind(), ErrorKind::Auth, "{err}");
    assert!(!oxi.is_retryable(), "needs a person: {oxi:?}");
    assert!(oxi.message().contains("Enter MFA code"));
}

#[tokio::test]
async fn never_policy_tells_the_plugin_it_is_not_interactive() {
    let dir = tempfile::tempdir().unwrap();
    // Succeeds only when kube reports `interactive: false` in KUBERNETES_EXEC_INFO.
    let script = format!(
        "case \"$KUBERNETES_EXEC_INFO\" in *'\"interactive\":false'*) {OK_CREDENTIAL};; *) exit 3;; esac\n"
    );
    let mut config = config_with_plugin(dir.path(), &script, None);
    ExecInteractivePolicy::Never
        .apply_to_config(&mut config)
        .unwrap();
    let err = connect(config)
        .await
        .expect_err("nothing listens on port 1");
    assert_eq!(
        classify(&err).kind(),
        ErrorKind::Network,
        "plugin must have run: {err}"
    );
}

#[tokio::test]
async fn always_plugin_under_never_policy_is_rejected_before_it_runs() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("ran");
    let script = format!("touch {}\n{OK_CREDENTIAL}\n", marker.display());
    let mut config = config_with_plugin(dir.path(), &script, Some(ExecInteractiveMode::Always));
    let err = ExecInteractivePolicy::Never
        .apply_to_config(&mut config)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Auth);
    assert!(!err.is_retryable());
    assert!(!marker.exists(), "the plugin must not have been executed");
}
