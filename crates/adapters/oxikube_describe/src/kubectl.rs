//! [`KubectlDescribe`]: `kubectl describe` as a child process.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use oxikube_domain::ids::{ContextName, ResourceRef};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{DescribeOutput, DescribePort, DescribeSource, DiscoveryPort};
use tokio::process::Command;

use crate::errors::classify;
use crate::preference::DescribePreference;
use crate::resolve::resolve_kind;

/// How long a `kubectl describe` may take before it is killed.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Which cluster `kubectl` talks to: a context name and, when the context is not in the files
/// `kubectl` reads by default, the kubeconfig file that defines it. Names and paths only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KubectlTarget {
    /// `--context`.
    pub context: ContextName,
    /// `--kubeconfig`, when the context's file is known.
    pub kubeconfig: Option<PathBuf>,
}

/// Runs `kubectl describe <plural[.group]> <name> [-n <namespace>]`.
///
/// The binary is the preference's `kubectl_path`, else `kubectl` on `PATH`. The child never
/// reads the terminal (stdin is closed), is killed when the call is dropped or after 30 s, and
/// inherits the environment (so exec credential plugins keep working); nothing secret is put on
/// its command line. A missing binary is `Unsupported`.
pub struct KubectlDescribe {
    discovery: Arc<dyn DiscoveryPort>,
    preference: DescribePreference,
    target: KubectlTarget,
}

impl std::fmt::Debug for KubectlDescribe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KubectlDescribe")
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

impl KubectlDescribe {
    /// A describer for `target`, finding the kind's plural through `discovery`.
    pub fn new(
        discovery: Arc<dyn DiscoveryPort>,
        preference: DescribePreference,
        target: KubectlTarget,
    ) -> Self {
        Self {
            discovery,
            preference,
            target,
        }
    }

    /// The arguments of the command line for `plural` (`deployments.apps`, `pods`).
    pub(crate) fn args(&self, resource: &str, target: &ResourceRef) -> Vec<String> {
        let mut args = vec!["--context".to_owned(), self.target.context.to_string()];
        if let Some(path) = &self.target.kubeconfig {
            args.push("--kubeconfig".to_owned());
            args.push(path.display().to_string());
        }
        args.push("describe".to_owned());
        args.push(resource.to_owned());
        args.push(target.name.to_string());
        if let Some(namespace) = target.namespace() {
            args.push("--namespace".to_owned());
            args.push(namespace.to_owned());
        }
        args
    }
}

#[async_trait]
impl DescribePort for KubectlDescribe {
    async fn describe(&self, target: &ResourceRef) -> OxiResult<DescribeOutput> {
        let kind = resolve_kind(self.discovery.as_ref(), &target.gvk).await?;
        let resource = if kind.gvk.group.is_empty() {
            kind.plural.clone()
        } else {
            format!("{}.{}", kind.plural, kind.gvk.group)
        };
        let binary = self
            .preference
            .get()
            .kubectl_path
            .unwrap_or_else(|| PathBuf::from("kubectl"));
        let mut command = Command::new(&binary);
        command
            .args(self.args(&resource, target))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let child = command.spawn().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                OxiError::unsupported(format!(
                    "kubectl was not found ({}); install it or set describe.kubectl_path",
                    binary.display()
                ))
            } else {
                OxiError::internal(format!("could not run kubectl: {error}"))
            }
        })?;
        let output = tokio::time::timeout(TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| OxiError::timeout("kubectl describe timed out"))?
            .map_err(|error| OxiError::internal(format!("kubectl failed: {error}")))?;
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            return Ok(DescribeOutput {
                text: text.into_owned(),
                source: DescribeSource::KubectlFallback,
            });
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = if stderr.trim().is_empty() {
            format!("kubectl exited with {}", output.status)
        } else {
            stderr.into_owned()
        };
        Err(classify(&stderr))
    }
}
