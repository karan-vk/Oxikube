//! Edge inputs: where the kubeconfig paths come from, and when to fall back to in-cluster.
//!
//! # Precedence
//!
//! 1. **Explicit sources** (settings, a flag): if any are given, only they are loaded.
//! 2. **`KUBECONFIG`**: split with [`split_kubeconfig`](super::split_kubeconfig); if it yields
//!    any path, only those are loaded. An unset or empty value skips this tier.
//! 3. **The default path** `<home>/.kube/config`.
//! 4. **In-cluster**: the pod's service account, as the synthetic `in-cluster` context
//!    ([`super::incluster`]).
//!
//! Tiers 1-3 are exclusive, as in kubectl: a `KUBECONFIG` whose files are all missing does not
//! fall through to the default path. Tier 4 is a last resort and invisible when a kubeconfig
//! gives any context. It runs only when no context was found **and** no listed file failed to
//! read, parse or merge: a broken kubeconfig is reported through diagnostics rather than
//! silently pointing the app at the pod's own cluster. A blank file or a missing file is not a
//! failure.
//!
//! [`Env`] carries everything the decision reads (variables and whether the service account
//! files are mounted), so tests inject it and never touch the process environment.
//! [`Env::from_process`] is the only reader of the real environment.

use std::ffi::OsString;
use std::path::PathBuf;

use oxikube_domain::{ErrorKind, OxiError, OxiResult};

use super::diagnostics::{Diagnostic, InClusterSkip, SourceInfo, SourceStatus, SourceTier};
use super::incluster::{
    IN_CLUSTER_SOURCE_LABEL, IN_CLUSTER_SOURCE_PATH, SERVICE_ACCOUNT_CA_FILE,
    SERVICE_ACCOUNT_NAMESPACE_FILE, SERVICE_ACCOUNT_TOKEN_FILE, in_cluster_context_name,
    in_cluster_kubeconfig, in_cluster_server_url,
};
use super::load::{load_kubeconfig_from_paths_blocking, unusable_error};
use super::split::{Platform, default_kubeconfig_path, split_kubeconfig_os};
use super::{LoadedKubeconfig, Strictness};

/// Everything the source decision reads from the outside world.
///
/// Holds no credentials (the service account token is never read), so `Debug` is safe.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// Which `KUBECONFIG` separator rules to apply.
    pub platform: Platform,
    /// The value of `KUBECONFIG`, if set.
    pub kubeconfig: Option<OsString>,
    /// The user's home directory, if known.
    pub home: Option<PathBuf>,
    /// `KUBERNETES_SERVICE_HOST`, if set.
    pub kubernetes_service_host: Option<String>,
    /// `KUBERNETES_SERVICE_PORT`, if set.
    pub kubernetes_service_port: Option<String>,
    /// Whether the service account token and CA files are mounted.
    pub service_account_mounted: bool,
    /// The pod's namespace from the mounted namespace file, if readable.
    pub service_account_namespace: Option<String>,
}

impl Env {
    /// Read the real process environment and check the service account files (blocking, tiny
    /// file checks; call it from a blocking context).
    pub fn from_process() -> Self {
        let var = |key: &str| std::env::var_os(key).filter(|v| !v.is_empty());
        let text = |key: &str| var(key).and_then(|v| v.into_string().ok());
        let mounted = std::path::Path::new(SERVICE_ACCOUNT_TOKEN_FILE).is_file()
            && std::path::Path::new(SERVICE_ACCOUNT_CA_FILE).is_file();
        let namespace = mounted
            .then(|| std::fs::read_to_string(SERVICE_ACCOUNT_NAMESPACE_FILE).ok())
            .flatten()
            .map(|ns| ns.trim().to_owned())
            .filter(|ns| !ns.is_empty());
        Self {
            platform: Platform::host(),
            kubeconfig: var("KUBECONFIG"),
            home: var("HOME")
                .or_else(|| var("USERPROFILE"))
                .map(PathBuf::from),
            kubernetes_service_host: text("KUBERNETES_SERVICE_HOST"),
            kubernetes_service_port: text("KUBERNETES_SERVICE_PORT"),
            service_account_mounted: mounted,
            service_account_namespace: namespace,
        }
    }

    /// True when this looks like a pod: service host and a valid port are set and the service
    /// account files are mounted. The same conditions `kube::Config::incluster()` needs.
    pub fn in_cluster_detected(&self) -> bool {
        match (&self.kubernetes_service_host, &self.kubernetes_service_port) {
            (Some(host), Some(port)) => {
                self.service_account_mounted && in_cluster_server_url(host, port).is_some()
            }
            _ => false,
        }
    }
}

/// The kubeconfig paths chosen by precedence and the tier they came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// The winning tier, or `None` when no tier gave a path.
    pub tier: Option<SourceTier>,
    /// The paths to load, in order.
    pub paths: Vec<PathBuf>,
}

/// Pick the kubeconfig paths to load: explicit sources, then `KUBECONFIG`, then the default
/// path (pure). Empty explicit paths are ignored.
pub fn select_sources(explicit: &[PathBuf], env: &Env) -> Selection {
    let explicit: Vec<PathBuf> = explicit
        .iter()
        .filter(|path| !path.as_os_str().is_empty())
        .cloned()
        .collect();
    if !explicit.is_empty() {
        return Selection {
            tier: Some(SourceTier::Explicit),
            paths: explicit,
        };
    }
    let from_env = env
        .kubeconfig
        .as_deref()
        .map(|value| split_kubeconfig_os(value, env.platform))
        .unwrap_or_default();
    if !from_env.is_empty() {
        return Selection {
            tier: Some(SourceTier::KubeconfigEnv),
            paths: from_env,
        };
    }
    match &env.home {
        Some(home) => Selection {
            tier: Some(SourceTier::DefaultPath),
            paths: vec![default_kubeconfig_path(home)],
        },
        None => Selection {
            tier: None,
            paths: Vec::new(),
        },
    }
}

/// True when a listed file exists but could not be used (read, parse or merge failure).
fn has_broken_source(loaded: &LoadedKubeconfig) -> bool {
    loaded.sources.iter().any(|s| {
        matches!(
            s.status,
            SourceStatus::Unreadable | SourceStatus::Unparsable | SourceStatus::Incompatible
        )
    })
}

/// Add the in-cluster context to `loaded`. It goes first in the merge, so its names win.
fn apply_in_cluster(loaded: &mut LoadedKubeconfig, env: &Env) -> OxiResult<()> {
    let synthetic = in_cluster_kubeconfig(env)?;
    let files = std::mem::take(&mut loaded.merged);
    loaded.merged = synthetic.merge(files).map_err(|err| {
        OxiError::internal("could not add the in-cluster context to the kubeconfig")
            .with_source(err)
    })?;
    let context = in_cluster_context_name();
    let path = PathBuf::from(IN_CLUSTER_SOURCE_PATH);
    loaded.origins.insert(context.clone(), path.clone());
    loaded.sources.push(SourceInfo {
        path,
        key: IN_CLUSTER_SOURCE_LABEL.to_owned(),
        status: SourceStatus::Loaded,
        contexts: vec![context],
    });
    Ok(())
}

/// Load the kubeconfig for `env` by the module's precedence rules (blocking).
///
/// Loads the selected tier tolerantly, then applies the in-cluster fallback when no context was
/// found and no file was broken. Diagnostics start with [`Diagnostic::SourceSelected`] (or
/// [`Diagnostic::NoKubeconfigSource`]) and, when no context was found, end with
/// [`Diagnostic::InClusterUsed`] or [`Diagnostic::InClusterSkipped`], so a log line says which
/// source won and whether the fallback ran. With [`Strictness::RequireUsable`] the load fails
/// when nothing usable remains: `NotFound` (nothing exists and not in a cluster) or
/// `Validation` (files exist but none is usable).
pub fn load_kubeconfig_for_env_blocking(
    explicit: &[PathBuf],
    env: &Env,
    strictness: Strictness,
) -> OxiResult<LoadedKubeconfig> {
    let selection = select_sources(explicit, env);
    let mut loaded = load_kubeconfig_from_paths_blocking(&selection.paths, Strictness::Tolerant)?;

    let fallback = if !loaded.merged.contexts.is_empty() {
        None
    } else if has_broken_source(&loaded) {
        Some(Diagnostic::InClusterSkipped {
            reason: InClusterSkip::BrokenKubeconfig,
        })
    } else if env.in_cluster_detected() {
        apply_in_cluster(&mut loaded, env)?;
        Some(Diagnostic::InClusterUsed)
    } else {
        Some(Diagnostic::InClusterSkipped {
            reason: InClusterSkip::NotInCluster,
        })
    };

    if strictness == Strictness::RequireUsable && !loaded.has_usable_source() {
        let err = unusable_error(&loaded);
        return Err(if err.kind() == ErrorKind::NotFound {
            OxiError::not_found(format!("{} (and not running in a cluster)", err.message()))
        } else {
            err
        });
    }

    let selected = match selection.tier {
        Some(tier) => Diagnostic::SourceSelected {
            tier,
            paths: selection.paths.len(),
        },
        None => Diagnostic::NoKubeconfigSource,
    };
    loaded.diagnostics.insert(0, selected);
    loaded.diagnostics.extend(fallback);
    Ok(loaded)
}

/// [`load_kubeconfig_for_env_blocking`] on tokio's blocking pool. Needs a tokio runtime.
pub async fn load_kubeconfig_for_env(
    explicit: Vec<PathBuf>,
    env: Env,
    strictness: Strictness,
) -> OxiResult<LoadedKubeconfig> {
    tokio::task::spawn_blocking(move || {
        load_kubeconfig_for_env_blocking(&explicit, &env, strictness)
    })
    .await
    .map_err(|err| OxiError::internal("kubeconfig loader task failed").with_source(err))?
}

/// Load the kubeconfig for the real process environment: reads [`Env::from_process`] and loads,
/// all off the calling thread. Needs a tokio runtime.
pub async fn load_kubeconfig_for_process(
    explicit: Vec<PathBuf>,
    strictness: Strictness,
) -> OxiResult<LoadedKubeconfig> {
    tokio::task::spawn_blocking(move || {
        load_kubeconfig_for_env_blocking(&explicit, &Env::from_process(), strictness)
    })
    .await
    .map_err(|err| OxiError::internal("kubeconfig loader task failed").with_source(err))?
}

#[cfg(test)]
mod tests;
