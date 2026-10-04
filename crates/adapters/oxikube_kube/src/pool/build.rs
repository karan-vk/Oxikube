//! Building a kube [`Client`] for one context.
//!
//! The path is split so later stories slot in without a rewrite:
//! [`build_config`] turns a [`ContextDefinition`] and a [`PoolConfig`] into a kube
//! [`Config`] (TLS flags and SOCKS5 handling from E03-S07 hook in here), and E03-S04's
//! [`auth::build_client`](crate::auth::build_client) turns that into a [`Client`] under
//! the [`ExecInteractivePolicy`](crate::auth::ExecInteractivePolicy) with its error
//! classification. [`ClientFactory`] is the seam the pool calls, so tests can count,
//! slow down or fail builds.
//!
//! Both steps block: `build_config` reads certificate files and `auth::build_client`
//! runs exec credential plugins synchronously (kube 4.2 `Auth::try_from`). The pool
//! calls the factory on `tokio::task::spawn_blocking` under
//! [`PoolConfig::exec_deadline`], which is why the factory itself is synchronous.

use kube::config::{KubeConfigOptions, KubeconfigError};
use kube::{Client, Config};
use oxikube_domain::OxiError;

use super::config::PoolConfig;
use super::entry::ContextDefinition;
use crate::auth::{build_client, classify_kubeconfig};

/// Builds clients for the pool. Called on a blocking thread inside a Tokio runtime.
pub trait ClientFactory: Send + Sync + 'static {
    /// Builds the client for `definition` with the pool's settings.
    fn build(
        &self,
        definition: &ContextDefinition,
        config: &PoolConfig,
    ) -> Result<Client, OxiError>;
}

/// The production factory: [`build_config`] then
/// [`auth::build_client`](crate::auth::build_client) with [`PoolConfig::exec_policy`].
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
        build_client(kube_config, config.exec_policy)
    }
}

/// The proxy-related environment, captured once and passed in explicitly so tests
/// never read or write process environment variables.
///
/// kube 4.2 already falls back to `HTTPS_PROXY` then `https_proxy` when a cluster has
/// no `proxy-url` (`ConfigLoader::proxy_url`), reading the process environment
/// directly. [`build_config`] recomputes the value from this struct and overwrites
/// kube's result, so the precedence is ours to test: the kubeconfig `proxy-url`
/// wins, then `HTTPS_PROXY`, then `https_proxy`. kube still consults the process
/// environment first, though: an unparseable `HTTPS_PROXY` there fails the build
/// before the override runs (reported as an invalid proxy URL, see
/// [`build_config`]). `NO_PROXY` is not honoured (kube does not either); see E03-S07.
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
///
/// Errors are classified by [`classify_kubeconfig`](crate::auth::classify_kubeconfig),
/// which never quotes credential material. An unparseable proxy URL, whether from the
/// cluster's `proxy-url`, from `proxy_env`, or from the process `HTTPS_PROXY` that kube
/// reads itself, is a [`Validation`](oxikube_domain::ErrorKind::Validation) error naming
/// those settings; the URL itself is never echoed (it may carry credentials).
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
    .map_err(|err| classify_kubeconfig(&err))?;

    config.connect_timeout = pool.connect_timeout;
    config.read_timeout = pool.read_timeout;
    config.write_timeout = pool.write_timeout;
    config.default_retry = pool.retry.default_retry();

    let proxy = cluster_proxy_url(definition).or_else(|| proxy_env.https_proxy.clone());
    config.proxy_url = match proxy {
        // Same classification as kube's own proxy parse; the URL is never echoed.
        Some(url) => Some(
            url.parse()
                .map_err(|err| classify_kubeconfig(&KubeconfigError::ParseProxyUrl(err)))?,
        ),
        None => None,
    };
    Ok(config)
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
