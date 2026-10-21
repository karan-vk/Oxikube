//! [`HelmCli`]: the [`HelmPort`](oxikube_ports::HelmPort) over the user's `helm` binary.
//!
//! Nobody reimplements Helm's templating in Rust, so repos, chart search, `show`, `template`,
//! install, upgrade, rollback and uninstall drive the real engine; the same binary also answers
//! the read methods (`list`, `history`, `get`) for a session that has no native reader.
//!
//! | File | Holds |
//! |---|---|
//! | `config` | [`HelmCliConfig`] (`helm.path` override), [`HelmTimeouts`] |
//! | `target` | [`HelmTarget`], [`TargetResolver`]: the kubeconfig context of a cluster |
//! | `detect` | finding the binary (override, `PATH`, usual directories) and the ≥ 3.10 gate |
//! | `args` | the argv of every command (`--flag=value`, positionals after `--`) |
//! | `run` | the child process: run to completion or stream lines; deadline; kill on drop |
//! | `parse` | the `-o json` parsers (`list`, `history`, `get values`, `repo list`, `search repo`) |
//! | `errors` | redaction of output and stderr; mapping failures onto the error taxonomy |
//! | `port` | the `HelmPort` implementation |
//! | `repos` | the `HelmRepoPort` implementation: `repo add/remove/update` on the user's own Helm configuration |
//!
//! # Detection
//!
//! [`detect`](HelmCli::detect) looks for the binary (the `helm.path` setting, else `PATH`,
//! else `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`), runs `helm version --short` and
//! requires [`HelmVersion::MINIMUM`](oxikube_domain::helm::HelmVersion::MINIMUM) (3.10). The
//! answer is cached: [`capabilities`](oxikube_ports::HelmPort::capabilities) reads it without
//! I/O, and the first operation detects when nothing has yet. Call
//! [`reconfigure`](HelmCli::reconfigure) when the settings change. A missing binary is
//! `BinaryMissing` and a too-old one `VersionTooOld` (both kind `Unsupported`), so callers hide
//! the CLI features rather than fail.
//!
//! # Cluster environment
//!
//! Every cluster command gets `--kube-context` and, when the context's file is known,
//! `--kubeconfig` (from the [`TargetResolver`]), plus `--namespace`. The `HELM_*` variables
//! that would override those (`HELM_NAMESPACE`, `HELM_KUBECONTEXT`, `HELM_KUBETOKEN`, ...) are
//! removed from the child's environment, so the user's shell state never picks the cluster.
//! The rest of the environment is inherited so exec credential plugins keep working.
//!
//! Credentials stay where they are: the adapter points `helm` at the user's own kubeconfig
//! file and writes no kubeconfig of its own. A context whose credentials exist only in memory
//! (a token kept in the keychain by the sources screen, the in-cluster context) has no file
//! `helm` can read; the binary leaves `kubeconfig` unset for it and `helm` then authenticates
//! with what its default kubeconfig says, or fails with `Auth`. That is a documented
//! limitation, chosen over writing a temporary kubeconfig with credentials (non-negotiable 5).
//!
//! # Values and secrets
//!
//! Values go to `helm` on stdin (`--values=-`), never on the command line (visible in `ps`)
//! and never in a temporary file. Output lines and error text are redacted: the string values
//! of the request are blanked and the shared patterns of `oxikube_domain::redact` applied.
//! Nothing here logs values, command output or stderr.
//!
//! # Processes
//!
//! Each command is a `tokio::process` child with `kill_on_drop(true)`: dropping the future (or
//! the install/upgrade output stream) kills it, and so does its deadline ([`HelmTimeouts`]),
//! which fails the call with `Timeout`. Callers run the futures on the runtime
//! (`oxikube_runtime::spawn_kube`), never on the UI thread.
//!
//! # Mutations
//!
//! `install`, `upgrade`, `rollback` and `uninstall` change the cluster. This adapter stays
//! dumb: it runs what it is asked. `oxikube_app` wraps each call in `MutationGuard`
//! (E16-S06/S09/S10); nothing else may call them. The [`HelmRepoPort`](oxikube_ports::HelmRepoPort)
//! methods change only the user's local Helm repository configuration (see `repos`).

mod args;
mod config;
mod detect;
mod errors;
mod parse;
mod port;
mod repos;
mod run;
mod target;
#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

use oxikube_domain::helm::{HelmCapabilities, HelmVersion};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use parking_lot::RwLock;

pub use config::{HelmCliConfig, HelmTimeouts};
pub use target::{HelmTarget, TargetResolver};

use detect::Detection;
use run::{Invocation, finish, run};

/// The `helm` CLI adapter. See the [module docs](self).
pub struct HelmCli {
    config: RwLock<HelmCliConfig>,
    detection: RwLock<Detection>,
    targets: Arc<dyn TargetResolver>,
}

impl std::fmt::Debug for HelmCli {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HelmCli")
            .field("config", &*self.config.read())
            .field("detection", &*self.detection.read())
            .finish_non_exhaustive()
    }
}

impl HelmCli {
    /// An adapter configured by `config`, finding each cluster's context through `targets`.
    /// Nothing runs until [`detect`](Self::detect) or the first operation.
    pub fn new(config: HelmCliConfig, targets: Arc<dyn TargetResolver>) -> Self {
        Self {
            config: RwLock::new(config),
            detection: RwLock::new(Detection::Undetected),
            targets,
        }
    }

    /// The current configuration.
    pub fn config(&self) -> HelmCliConfig {
        self.config.read().clone()
    }

    /// Finds the binary and checks its version now, caching the answer for
    /// [`capabilities`](oxikube_ports::HelmPort::capabilities).
    pub async fn detect(&self) -> HelmCapabilities {
        let config = self.config();
        let detection = detect::detect(&config).await;
        let capabilities = detection.capabilities();
        *self.detection.write() = detection;
        capabilities
    }

    /// Applies new settings (for example a changed `helm.path`) and detects again.
    pub async fn reconfigure(&self, config: HelmCliConfig) -> HelmCapabilities {
        *self.config.write() = config;
        self.detect().await
    }

    /// The binary and its version, detecting first when nothing has been detected yet.
    async fn binary(&self) -> OxiResult<(PathBuf, HelmVersion)> {
        if *self.detection.read() == Detection::Undetected {
            self.detect().await;
        }
        let detection = self.detection.read();
        detection
            .binary()
            .map(|(path, version)| (path.to_path_buf(), version))
    }

    fn target(&self, cluster: &ClusterId) -> OxiResult<HelmTarget> {
        self.targets.target(cluster).ok_or_else(|| {
            OxiError::not_found(format!(
                "no kubeconfig context is known for cluster {cluster}"
            ))
        })
    }

    /// Runs a read-only (or local-config) command with deadline `timeout` and returns stdout.
    async fn output(
        &self,
        args: Vec<String>,
        timeout: std::time::Duration,
        release: Option<&str>,
    ) -> OxiResult<String> {
        let (binary, _) = self.binary().await?;
        finish(run(&binary, Invocation::new(args, timeout)).await, release)
    }
}
