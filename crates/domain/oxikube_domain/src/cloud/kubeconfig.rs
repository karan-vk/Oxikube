//! [`KubeconfigEntry`]: the exec-auth kubeconfig context that connects to a discovered
//! cluster.

use serde::{Deserialize, Serialize};

use crate::ids::ContextName;

/// The `apiVersion` of the exec credential every generated entry asks for: the version
/// `aws eks get-token`, `gke-gcloud-auth-plugin` and `kubelogin` all speak.
pub const EXEC_API_VERSION: &str = "client.authentication.k8s.io/v1beta1";

/// kubeconfig's `interactiveMode` for an exec plugin: whether it may prompt (an MFA code, a
/// device-code sign-in). Generated entries always set it, so kube clients never guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecInteractiveMode {
    /// The plugin never prompts.
    Never,
    /// The plugin may prompt when a terminal is available.
    IfAvailable,
    /// The plugin needs a terminal.
    Always,
}

impl ExecInteractiveMode {
    /// The kubeconfig spelling (`Never`, `IfAvailable`, `Always`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Never => "Never",
            Self::IfAvailable => "IfAvailable",
            Self::Always => "Always",
        }
    }
}

/// One `env` entry of an exec plugin (`AWS_PROFILE=prod`). Never a secret: profiles, regions
/// and config switches only.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExecEnvVar {
    /// Variable name.
    pub name: String,
    /// Its value.
    pub value: String,
}

/// The exec credential plugin of an entry: the command a kube client runs to get a
/// short-lived token (`aws eks get-token --cluster-name prod --region eu-west-1`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExecPlugin {
    /// The exec credential `apiVersion`, normally [`EXEC_API_VERSION`].
    pub api_version: String,
    /// The program (`aws`, `gke-gcloud-auth-plugin`, `kubelogin`), a bare name resolved on
    /// `PATH` or an absolute path.
    pub command: String,
    /// Its arguments.
    pub args: Vec<String>,
    /// Extra environment for it.
    pub env: Vec<ExecEnvVar>,
    /// Whether it may prompt.
    pub interactive_mode: ExecInteractiveMode,
    /// Shown by kube clients when `command` is missing (`installHint`).
    pub install_hint: Option<String>,
    /// kubeconfig's `provideClusterInfo`.
    pub provide_cluster_info: bool,
}

impl ExecPlugin {
    /// A plugin running `command args` with [`EXEC_API_VERSION`], no env, no install hint and
    /// `interactive_mode`.
    pub fn new(
        command: impl Into<String>,
        args: Vec<String>,
        interactive_mode: ExecInteractiveMode,
    ) -> Self {
        Self {
            api_version: EXEC_API_VERSION.to_owned(),
            command: command.into(),
            args,
            env: Vec::new(),
            interactive_mode,
            install_hint: None,
            provide_cluster_info: false,
        }
    }
}

/// A kubeconfig context for one discovered cluster: its own cluster (server + CA) and user
/// (the exec plugin), under one context name. `CloudDiscoveryPort::kubeconfig_entry` builds it
/// and the discovery service writes it into an oxikube-managed kubeconfig file.
///
/// There is deliberately no token, password or client-key field: the only way an entry
/// authenticates is [`exec`](Self::exec).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KubeconfigEntry {
    /// The context name, also used for the entry's cluster and user names
    /// (`eks-prod-eu-west-1-prod`).
    pub context: ContextName,
    /// API server URL.
    pub server: String,
    /// The cluster CA, base64 PEM (`certificate-authority-data`); `None` uses the system
    /// roots.
    pub ca_data: Option<String>,
    /// How the user authenticates.
    pub exec: ExecPlugin,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugins_default_to_the_v1beta1_credential() {
        let plugin = ExecPlugin::new(
            "aws",
            vec!["eks".into(), "get-token".into()],
            ExecInteractiveMode::IfAvailable,
        );
        assert_eq!(plugin.api_version, EXEC_API_VERSION);
        assert!(plugin.env.is_empty() && plugin.install_hint.is_none());
        assert_eq!(plugin.interactive_mode.as_str(), "IfAvailable");
        assert_eq!(ExecInteractiveMode::Never.as_str(), "Never");
        assert_eq!(ExecInteractiveMode::Always.as_str(), "Always");
    }

    #[test]
    fn entries_round_trip_through_serde() {
        let entry = KubeconfigEntry {
            context: ContextName::new("gke-p-europe-west1-c"),
            server: "https://203.0.113.10".into(),
            ca_data: None,
            exec: ExecPlugin::new(
                "gke-gcloud-auth-plugin",
                Vec::new(),
                ExecInteractiveMode::Never,
            ),
        };
        let back: KubeconfigEntry =
            serde_json::from_str(&serde_json::to_string(&entry).unwrap()).unwrap();
        assert_eq!(back, entry);
    }
}
