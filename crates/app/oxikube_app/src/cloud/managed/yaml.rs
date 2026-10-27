//! The YAML of a managed kubeconfig: [`ManagedKubeconfig`], read and written as plain
//! kubeconfig (`clusters`, `contexts`, `users`), one cluster, context and user per entry, all
//! three named after the entry's context.
//!
//! The user is an exec plugin only (`aws eks get-token`, `gke-gcloud-auth-plugin`,
//! `kubelogin`) with `interactiveMode` set; there is no field for a token or a client key, so
//! none can be written. Each context carries the discovered cluster's key in an
//! `oxikube.dev/cloud` extension, so the catalog can tell which discovered clusters were added.

use oxikube_domain::cloud::kubeconfig::EXEC_API_VERSION;
use oxikube_domain::ids::ContextName;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ExecEnvVar, ExecInteractiveMode, ExecPlugin, KubeconfigEntry};
use serde::{Deserialize, Serialize};

/// The first line of every managed file. A file at a managed path without it was not written by
/// cloud discovery and is never overwritten.
pub(crate) const MARKER: &str =
    "# Managed by Oxikube cloud discovery. Edits are replaced when clusters are added or removed.";

/// The name of the context extension holding the discovered cluster's key.
const EXTENSION: &str = "oxikube.dev/cloud";

/// One cluster of a managed file: its kubeconfig entry and the discovered cluster's key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedEntry {
    /// The kubeconfig entry.
    pub entry: KubeconfigEntry,
    /// The [`DiscoveredCluster::key`](oxikube_ports::DiscoveredCluster::key) it was made from,
    /// when the file records it.
    pub cluster_key: Option<String>,
}

/// The entries of one managed file, in context-name order (so a rewrite is deterministic).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ManagedKubeconfig {
    entries: Vec<ManagedEntry>,
}

impl ManagedKubeconfig {
    pub fn entries(&self) -> &[ManagedEntry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Adds `entry`, replacing one with the same context. Returns whether the file changed.
    pub fn upsert(&mut self, entry: ManagedEntry) -> bool {
        match self
            .entries
            .iter_mut()
            .find(|e| e.entry.context == entry.entry.context)
        {
            Some(existing) if *existing == entry => false,
            Some(existing) => {
                *existing = entry;
                true
            }
            None => {
                self.entries.push(entry);
                self.entries
                    .sort_by(|a, b| a.entry.context.as_str().cmp(b.entry.context.as_str()));
                true
            }
        }
    }

    /// Removes the entry of `context`. Returns whether there was one.
    pub fn remove(&mut self, context: &ContextName) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.entry.context != *context);
        self.entries.len() != before
    }

    /// Parses a managed file. A context whose cluster or user is missing, or whose user is not
    /// an exec plugin, is skipped.
    pub fn parse(text: &str) -> OxiResult<Self> {
        let doc: Doc = serde_saphyr::from_str(text).map_err(|error| {
            OxiError::validation(format!("the managed kubeconfig cannot be read: {error}"))
        })?;
        let mut file = Self::default();
        for named in doc.contexts {
            let cluster = doc
                .clusters
                .iter()
                .find(|c| c.name == named.context.cluster);
            let user = doc.users.iter().find(|u| u.name == named.context.user);
            let (Some(cluster), Some(UserDoc { exec: Some(exec) })) =
                (cluster, user.map(|u| &u.user))
            else {
                continue;
            };
            let cluster_key = named
                .context
                .extensions
                .iter()
                .find(|e| e.name == EXTENSION)
                .map(|e| e.extension.key.clone());
            file.upsert(ManagedEntry {
                entry: KubeconfigEntry {
                    context: ContextName::new(named.name),
                    server: cluster.cluster.server.clone(),
                    ca_data: cluster.cluster.certificate_authority_data.clone(),
                    exec: exec.to_plugin(),
                },
                cluster_key,
            });
        }
        Ok(file)
    }

    /// The file's text: the [`MARKER`] line, then the kubeconfig.
    pub fn render(&self) -> OxiResult<String> {
        let doc = Doc {
            api_version: "v1".to_owned(),
            kind: "Config".to_owned(),
            clusters: self.entries.iter().map(cluster_doc).collect(),
            contexts: self.entries.iter().map(context_doc).collect(),
            users: self.entries.iter().map(user_doc).collect(),
        };
        let yaml = serde_saphyr::to_string(&doc).map_err(|error| {
            OxiError::internal(format!("the managed kubeconfig cannot be written: {error}"))
        })?;
        Ok(format!("{MARKER}\n{yaml}"))
    }
}

fn cluster_doc(e: &ManagedEntry) -> NamedCluster {
    NamedCluster {
        name: e.entry.context.as_str().to_owned(),
        cluster: ClusterDoc {
            server: e.entry.server.clone(),
            certificate_authority_data: e.entry.ca_data.clone(),
        },
    }
}

fn context_doc(e: &ManagedEntry) -> NamedContext {
    let name = e.entry.context.as_str().to_owned();
    NamedContext {
        context: ContextDoc {
            cluster: name.clone(),
            user: name.clone(),
            extensions: e
                .cluster_key
                .iter()
                .map(|key| NamedExtension {
                    name: EXTENSION.to_owned(),
                    extension: CloudExtension { key: key.clone() },
                })
                .collect(),
        },
        name,
    }
}

fn user_doc(e: &ManagedEntry) -> NamedUser {
    let exec = &e.entry.exec;
    NamedUser {
        name: e.entry.context.as_str().to_owned(),
        user: UserDoc {
            exec: Some(ExecDoc {
                api_version: exec.api_version.clone(),
                command: exec.command.clone(),
                args: exec.args.clone(),
                env: exec
                    .env
                    .iter()
                    .map(|v| EnvDoc {
                        name: v.name.clone(),
                        value: v.value.clone(),
                    })
                    .collect(),
                interactive_mode: exec.interactive_mode.as_str().to_owned(),
                provide_cluster_info: exec.provide_cluster_info,
                install_hint: exec.install_hint.clone(),
            }),
        },
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Doc {
    #[serde(rename = "apiVersion")]
    api_version: String,
    kind: String,
    #[serde(default)]
    clusters: Vec<NamedCluster>,
    #[serde(default)]
    contexts: Vec<NamedContext>,
    #[serde(default)]
    users: Vec<NamedUser>,
}

#[derive(Debug, Serialize, Deserialize)]
struct NamedCluster {
    name: String,
    cluster: ClusterDoc,
}

#[derive(Debug, Serialize, Deserialize)]
struct ClusterDoc {
    server: String,
    #[serde(
        rename = "certificate-authority-data",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    certificate_authority_data: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct NamedContext {
    name: String,
    context: ContextDoc,
}

#[derive(Debug, Serialize, Deserialize)]
struct ContextDoc {
    cluster: String,
    user: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    extensions: Vec<NamedExtension>,
}

#[derive(Debug, Serialize, Deserialize)]
struct NamedExtension {
    name: String,
    extension: CloudExtension,
}

#[derive(Debug, Serialize, Deserialize)]
struct CloudExtension {
    key: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct NamedUser {
    name: String,
    user: UserDoc,
}

#[derive(Debug, Serialize, Deserialize)]
struct UserDoc {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exec: Option<ExecDoc>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecDoc {
    #[serde(default = "default_api_version")]
    api_version: String,
    command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    args: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    env: Vec<EnvDoc>,
    #[serde(default = "default_interactive_mode")]
    interactive_mode: String,
    #[serde(default)]
    provide_cluster_info: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    install_hint: Option<String>,
}

impl ExecDoc {
    fn to_plugin(&self) -> ExecPlugin {
        let interactive_mode = match self.interactive_mode.as_str() {
            "Never" => ExecInteractiveMode::Never,
            "Always" => ExecInteractiveMode::Always,
            _ => ExecInteractiveMode::IfAvailable,
        };
        ExecPlugin {
            api_version: self.api_version.clone(),
            command: self.command.clone(),
            args: self.args.clone(),
            env: self
                .env
                .iter()
                .map(|v| ExecEnvVar {
                    name: v.name.clone(),
                    value: v.value.clone(),
                })
                .collect(),
            interactive_mode,
            install_hint: self.install_hint.clone(),
            provide_cluster_info: self.provide_cluster_info,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct EnvDoc {
    name: String,
    value: String,
}

fn default_api_version() -> String {
    EXEC_API_VERSION.to_owned()
}

fn default_interactive_mode() -> String {
    ExecInteractiveMode::IfAvailable.as_str().to_owned()
}
