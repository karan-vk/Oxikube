//! One run of an exec plugin that returns a client certificate (E03-F550).
//!
//! kube-client 4.2 keeps the exec machinery private. A plugin that returns a client
//! certificate is read from the config three times while a client is built (the identity's
//! expiry, the TLS identity, the auth layer), each time by running the plugin. This module
//! runs the plugin once itself, with the same invocation kube uses, so the caller can move
//! the certificate and key into the config's inline `client-certificate-data` /
//! `client-key-data` and clear `exec`: kube then finds a static identity and runs nothing.
//!
//! The invocation matches kube's `auth_exec`: the configured `command` and `args`, the
//! plugin's `env` entries, `KUBERNETES_EXEC_INFO` (api version, `interactive`, and the
//! cluster when `provideClusterInfo` is set), `drop_env`, stdin and stderr inherited unless
//! `interactiveMode` is `Never` (stdin is then a pipe that is closed at once), and stdout
//! parsed as JSON or YAML. Failures are reported as the kube errors the rest of the adapter
//! already classifies, so a failed run reads the same whichever path ran the plugin.
//!
//! The key is held only in memory: a `String` while it is moved into the config's
//! `SecretString`, never logged, never in an error.

use std::process::{Command, Stdio};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use jiff::Timestamp;
use kube::client::AuthError;
use kube::config::{AuthInfo, ExecAuthCluster, ExecConfig, ExecInteractiveMode};
use oxikube_domain::{OxiError, OxiResult};
use serde::{Deserialize, Serialize};

use super::classify::{CredentialRefresh, classify_with};

/// What a plugin run handed back.
pub(super) enum PluginOutput {
    /// A client certificate and key (PEM), and when they expire.
    Identity(Identity),
    /// A bearer token. Not kept: the token path is kube's own refreshable one.
    Token,
    /// A `status` with neither a token nor a certificate.
    Nothing,
}

/// The client certificate a plugin returned.
pub(super) struct Identity {
    certificate: String,
    key: String,
    /// `expirationTimestamp`, which kube reports as `Client::valid_until`.
    pub(super) expires: Option<Timestamp>,
}

impl Identity {
    /// Puts the identity into `info` as inline kubeconfig data and clears `exec`, so the
    /// plugin is not run again. Inline data wins over `client-certificate` / `client-key`
    /// files, which are cleared anyway: exec takes precedence over them in kube too.
    pub(super) fn install(self, info: &mut AuthInfo) {
        info.exec = None;
        info.client_certificate = None;
        info.client_key = None;
        info.client_certificate_data = Some(STANDARD.encode(self.certificate));
        info.client_key_data = Some(STANDARD.encode(self.key).into());
    }
}

#[derive(Serialize)]
struct ExecInfo<'a> {
    kind: &'static str,
    #[serde(rename = "apiVersion", skip_serializing_if = "Option::is_none")]
    api_version: Option<&'a str>,
    spec: ExecInfoSpec<'a>,
}

#[derive(Serialize)]
struct ExecInfoSpec<'a> {
    interactive: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    cluster: Option<&'a ExecAuthCluster>,
}

#[derive(Deserialize)]
struct Credential {
    status: Option<Status>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    expiration_timestamp: Option<String>,
    token: Option<serde::de::IgnoredAny>,
    client_certificate_data: Option<String>,
    client_key_data: Option<String>,
}

/// Runs the plugin of `exec` once and reads what it returned. Blocks until it exits.
///
/// # Errors
///
/// The failure classified as kube's own run of the plugin would be: the plugin could not
/// start, exited non-zero, printed something that is not an `ExecCredential`, or returned an
/// invalid `expirationTimestamp`.
pub(super) fn run_once(exec: &ExecConfig, refresh: CredentialRefresh) -> OxiResult<PluginOutput> {
    let fail = |err: AuthError| classify_with(&kube::Error::Auth(err), refresh);
    let stdout = run_plugin(exec).map_err(fail)?;
    let credential: Credential = serde_saphyr::from_slice(&stdout).map_err(|_| {
        OxiError::auth(
            "the exec credential plugin printed output that is not a valid ExecCredential",
            false,
        )
    })?;
    let status = credential
        .status
        .ok_or_else(|| fail(AuthError::ExecPluginFailed))?;
    let expires = status
        .expiration_timestamp
        .map(|ts| ts.parse::<Timestamp>())
        .transpose()
        .map_err(|e| fail(AuthError::MalformedTokenExpirationDate(e)))?;
    Ok(
        match (status.client_certificate_data, status.client_key_data) {
            (Some(certificate), Some(key)) => PluginOutput::Identity(Identity {
                certificate,
                key,
                expires,
            }),
            _ if status.token.is_some() => PluginOutput::Token,
            _ => PluginOutput::Nothing,
        },
    )
}

/// Starts the plugin like kube's `auth_exec` and returns its stdout.
fn run_plugin(exec: &ExecConfig) -> Result<Vec<u8>, AuthError> {
    let mut cmd = Command::new(exec.command.as_ref().ok_or(AuthError::MissingCommand)?);
    if let Some(args) = &exec.args {
        cmd.args(args);
    }
    for entry in exec.env.iter().flatten() {
        if let (Some(name), Some(value)) = (entry.get("name"), entry.get("value")) {
            cmd.env(name, value);
        }
    }

    let interactive = exec.interactive_mode != Some(ExecInteractiveMode::Never);
    if interactive {
        cmd.stdin(Stdio::inherit()).stderr(Stdio::inherit());
    } else {
        cmd.stdin(Stdio::piped());
    }

    let cluster = if exec.provide_cluster_info {
        Some(
            exec.cluster
                .as_ref()
                .ok_or(AuthError::ExecMissingClusterInfo)?,
        )
    } else {
        None
    };
    let info = serde_json::to_string(&ExecInfo {
        kind: "ExecCredential",
        api_version: exec.api_version.as_deref(),
        spec: ExecInfoSpec {
            interactive,
            cluster,
        },
    })
    .map_err(AuthError::AuthExecSerialize)?;
    cmd.env("KUBERNETES_EXEC_INFO", info);
    for name in exec.drop_env.iter().flatten() {
        cmd.env_remove(name);
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // Opt-in, as in kube: CREATE_NO_WINDOW breaks stderr inheritance for interactive
        // plugins such as kubelogin.exe.
        if std::env::var("KUBE_RS_UNSTABLE_CREATE_NO_WINDOW").is_ok_and(|s| s == "1") {
            cmd.creation_flags(0x0800_0000);
        }
    }

    let out = cmd.output().map_err(AuthError::AuthExecStart)?;
    if !out.status.success() {
        return Err(AuthError::AuthExecRun {
            cmd: format!("{cmd:?}"),
            status: out.status,
            out,
        });
    }
    Ok(out.stdout)
}

#[cfg(all(test, unix))]
mod tests {
    use std::collections::HashMap;

    use oxikube_domain::ErrorKind;

    use super::*;

    const STATUS_OK: &str = r#"{"status":{"clientCertificateData":"CERT","clientKeyData":"KEY","expirationTimestamp":"2099-01-01T00:00:00Z"}}"#;

    /// An exec config running `sh -c <script>`, not interactive.
    fn sh(script: &str) -> ExecConfig {
        ExecConfig {
            command: Some("sh".into()),
            args: Some(vec!["-c".into(), script.into()]),
            api_version: Some("client.authentication.k8s.io/v1".into()),
            interactive_mode: Some(ExecInteractiveMode::Never),
            ..Default::default()
        }
    }

    fn echo(json: &str) -> ExecConfig {
        sh(&format!("printf '%s' '{json}'"))
    }

    fn refresh() -> CredentialRefresh {
        CredentialRefresh::of(&AuthInfo {
            exec: Some(ExecConfig::default()),
            ..AuthInfo::default()
        })
    }

    fn run(exec: &ExecConfig) -> OxiResult<PluginOutput> {
        run_once(exec, refresh())
    }

    #[test]
    fn a_certificate_and_key_are_an_identity_with_its_expiry() {
        let Ok(PluginOutput::Identity(id)) = run(&echo(STATUS_OK)) else {
            panic!("expected an identity");
        };
        assert_eq!(id.expires.unwrap().to_string(), "2099-01-01T00:00:00Z");
        let mut info = AuthInfo {
            exec: Some(ExecConfig::default()),
            client_certificate: Some("/old/cert".into()),
            ..AuthInfo::default()
        };
        id.install(&mut info);
        assert!(info.exec.is_none() && info.client_certificate.is_none());
        let cert = info.client_certificate_data.unwrap();
        assert_eq!(STANDARD.decode(cert).unwrap(), b"CERT");
        assert!(info.client_key_data.is_some());
    }

    #[test]
    fn yaml_output_is_read_like_json() {
        let yaml = "apiVersion: client.authentication.k8s.io/v1\nstatus:\n  clientCertificateData: C\n  clientKeyData: K\n";
        let exec = sh(&format!("printf '%s' '{yaml}'"));
        assert!(matches!(run(&exec), Ok(PluginOutput::Identity(_))));
    }

    #[test]
    fn a_token_or_nothing_is_not_an_identity() {
        assert!(matches!(
            run(&echo(r#"{"status":{"token":"T"}}"#)),
            Ok(PluginOutput::Token)
        ));
        assert!(matches!(
            run(&echo(r#"{"status":{}}"#)),
            Ok(PluginOutput::Nothing)
        ));
        // Half an identity is not one.
        assert!(matches!(
            run(&echo(r#"{"status":{"clientCertificateData":"C"}}"#)),
            Ok(PluginOutput::Nothing)
        ));
    }

    #[test]
    fn bad_output_is_a_permanent_auth_error() {
        for exec in [
            echo("not an ExecCredential: ["),
            echo(r#"{"status":{"clientCertificateData":"C","expirationTimestamp":"soon"}}"#),
        ] {
            let Err(err) = run(&exec) else {
                panic!("expected an error");
            };
            assert_eq!(err.kind(), ErrorKind::Auth, "{err}");
            assert!(!err.is_retryable(), "{err}");
        }
    }

    #[test]
    fn a_missing_status_and_a_failing_plugin_are_retryable() {
        let Err(err) = run(&echo("{}")) else {
            panic!("no status");
        };
        assert_eq!(err.kind(), ErrorKind::Auth);
        let Err(err) = run(&sh("echo boom >&2; exit 4")) else {
            panic!("exit 4");
        };
        assert!(err.message().contains("boom"), "{err}");
    }

    #[test]
    fn a_missing_command_is_incomplete_configuration() {
        let exec = ExecConfig::default();
        let Err(err) = run(&exec) else {
            panic!("no command");
        };
        assert!(!err.is_retryable(), "{err}");
    }

    #[test]
    fn the_plugin_gets_kubes_environment() {
        // Exits 0 with a certificate only when every expectation holds.
        let mut exec = sh(concat!(
            "case \"$KUBERNETES_EXEC_INFO\" in ",
            "*'\"interactive\":false'*'\"cluster\"'*) ;; *) exit 3;; esac; ",
            "[ \"$FROM_CONFIG\" = yes ] || exit 4; ",
            "[ -z \"$DROPPED\" ] || exit 5; ",
            "printf '%s' '{\"status\":{\"clientCertificateData\":\"C\",\"clientKeyData\":\"K\"}}'"
        ));
        exec.provide_cluster_info = true;
        exec.cluster = Some(ExecAuthCluster::default());
        exec.env = Some(vec![HashMap::from([
            ("name".to_owned(), "FROM_CONFIG".to_owned()),
            ("value".to_owned(), "yes".to_owned()),
        ])]);
        exec.drop_env = Some(vec!["DROPPED".into()]);
        assert!(matches!(run(&exec), Ok(PluginOutput::Identity(_))));
    }

    #[test]
    fn provide_cluster_info_without_a_cluster_is_incomplete_configuration() {
        let mut exec = echo(STATUS_OK);
        exec.provide_cluster_info = true;
        let Err(err) = run(&exec) else {
            panic!("no cluster");
        };
        assert!(!err.is_retryable(), "{err}");
    }
}
