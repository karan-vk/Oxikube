//! Building a kube [`Client`] for one context.
//!
//! The path is split so later stories slot in without a rewrite:
//! [`build_config`] turns a [`ContextDefinition`] and a [`PoolConfig`] into a kube
//! [`Config`] (TLS flags and SOCKS5 handling from E03-S07 hook in here), and
//! [`build_client`] turns that into a [`Client`] (exec-plugin policy from E03-S04
//! hooks in here). [`ClientFactory`] is the seam the pool calls, so tests can count
//! or fail builds.
//!
//! Both steps block: `build_config` reads certificate files and `build_client` runs
//! exec credential plugins synchronously (kube 4.2 `Auth::try_from`). The pool
//! calls the factory on `tokio::task::spawn_blocking`.

use kube::config::{KubeConfigOptions, KubeconfigError};
use kube::{Client, Config};
use oxikube_domain::OxiError;

use super::config::PoolConfig;
use super::entry::ContextDefinition;

/// Builds clients for the pool. Called on a blocking thread inside a Tokio runtime.
pub trait ClientFactory: Send + Sync + 'static {
    /// Builds the client for `definition` with the pool's settings.
    fn build(
        &self,
        definition: &ContextDefinition,
        config: &PoolConfig,
    ) -> Result<Client, OxiError>;
}

/// The production factory: [`build_config`] then [`build_client`].
#[derive(Debug, Clone, Default)]
pub struct KubeClientFactory {
    proxy_env: ProxyEnv,
}

impl KubeClientFactory {
    /// A factory that falls back to the process's `HTTPS_PROXY` / `https_proxy`.
    pub fn from_process_env() -> Self {
        Self::new(ProxyEnv::from_process())
    }

    /// A factory with an explicit proxy environment (tests, or a settings override).
    pub fn new(proxy_env: ProxyEnv) -> Self {
        Self { proxy_env }
    }
}

impl ClientFactory for KubeClientFactory {
    fn build(
        &self,
        definition: &ContextDefinition,
        config: &PoolConfig,
    ) -> Result<Client, OxiError> {
        let kube_config = build_config(definition, config, &self.proxy_env)?;
        build_client(kube_config, definition)
    }
}

/// The proxy-related environment, captured once and passed in explicitly so tests
/// never read or write process environment variables.
///
/// kube 4.2 already falls back to `HTTPS_PROXY` then `https_proxy` when a cluster has
/// no `proxy-url` (`ConfigLoader::proxy_url`), reading the process environment
/// directly. [`build_config`] recomputes the value from this struct and overwrites
/// kube's result, so the outcome is deterministic and the precedence is ours to
/// test: the kubeconfig `proxy-url` wins, then `HTTPS_PROXY`, then `https_proxy`.
/// `NO_PROXY` is not honoured (kube does not either); see E03-S07.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ProxyEnv {
    https_proxy: Option<String>,
}

impl ProxyEnv {
    /// Reads `HTTPS_PROXY`, then `https_proxy`. Empty values count as unset.
    pub fn from_process() -> Self {
        let read = |key| std::env::var(key).ok().filter(|v: &String| !v.is_empty());
        Self {
            https_proxy: read("HTTPS_PROXY").or_else(|| read("https_proxy")),
        }
    }

    /// An explicit fallback proxy URL, or `None` for no fallback.
    pub fn with_https_proxy(https_proxy: Option<String>) -> Self {
        Self {
            https_proxy: https_proxy.filter(|v| !v.is_empty()),
        }
    }
}

impl std::fmt::Debug for ProxyEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A proxy URL can carry `user:password@`; print only whether one is set.
        f.debug_struct("ProxyEnv")
            .field("https_proxy_set", &self.https_proxy.is_some())
            .finish()
    }
}

/// Builds the kube [`Config`] for `definition`: kube's own kubeconfig resolution
/// (`Config::from_custom_kubeconfig`), then the pool's timeouts, retry mode and
/// proxy precedence.
///
/// Compression stays as the kubeconfig says (`disable-compression`, default off).
/// With the workspace's `gzip` kube feature the client then sends
/// `Accept-Encoding: gzip` and decodes responses.
pub fn build_config(
    definition: &ContextDefinition,
    pool: &PoolConfig,
    proxy_env: &ProxyEnv,
) -> Result<Config, OxiError> {
    let context = definition.context();
    let options = KubeConfigOptions {
        context: Some(context.as_str().to_owned()),
        ..KubeConfigOptions::default()
    };
    // `from_custom_kubeconfig` is `async` but never awaits (kube 4.2 resolves the
    // kubeconfig synchronously), so `block_on` returns at once. We are on a blocking
    // thread (see the module docs), never on an async worker.
    let mut config = futures::executor::block_on(Config::from_custom_kubeconfig(
        definition.kubeconfig().clone(),
        &options,
    ))
    .map_err(|err| kubeconfig_error(context.as_str(), err))?;

    config.connect_timeout = pool.connect_timeout;
    config.read_timeout = pool.read_timeout;
    config.write_timeout = pool.write_timeout;
    config.default_retry = pool.retry.default_retry();

    let proxy = cluster_proxy_url(definition).or_else(|| proxy_env.https_proxy.clone());
    config.proxy_url = match proxy {
        // The URL may hold credentials; keep it out of the message.
        Some(url) => Some(url.parse().map_err(|_| {
            OxiError::validation(format!(
                "context `{context}`: the proxy URL is not a valid URI"
            ))
        })?),
        None => None,
    };
    Ok(config)
}

/// Builds the client. Runs exec credential plugins, so it blocks.
///
/// kube's errors here can quote exec-plugin output or a proxy URL with
/// credentials, so the source is not attached and the message is fixed. E03-S04
/// classifies auth failures in detail; E03-S08 adds redaction, after which a
/// redacted source can be attached.
pub fn build_client(config: Config, definition: &ContextDefinition) -> Result<Client, OxiError> {
    let context = definition.context();
    Client::try_from(config).map_err(|err| match err {
        kube::Error::Auth(_) => OxiError::auth(
            format!("context `{context}`: could not obtain credentials"),
            true,
        ),
        kube::Error::ProxyProtocolUnsupported { .. }
        | kube::Error::ProxyProtocolDisabled { .. } => {
            OxiError::unsupported(format!("context `{context}`: unsupported proxy scheme"))
        }
        _ => OxiError::internal(format!("context `{context}`: could not create the client")),
    })
}

/// The cluster's own `proxy-url`, when set and non-empty.
fn cluster_proxy_url(definition: &ContextDefinition) -> Option<String> {
    definition
        .kubeconfig()
        .clusters
        .first()?
        .cluster
        .as_ref()?
        .proxy_url
        .clone()
        .filter(|url| !url.is_empty())
}

/// Maps kube's kubeconfig errors. Their messages carry paths and parse errors but
/// no credential values, so the source is kept.
fn kubeconfig_error(context: &str, err: KubeconfigError) -> OxiError {
    let base = match err {
        KubeconfigError::LoadContext(_) => {
            OxiError::not_found(format!("context `{context}` has no definition"))
        }
        _ => OxiError::validation(format!("context `{context}`: invalid kubeconfig entry")),
    };
    base.with_source(err)
}
