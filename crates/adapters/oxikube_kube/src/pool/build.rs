//! Building a kube [`Client`] for one context.
//!
//! The path is split so later stories slot in without a rewrite:
//! [`build_config`] turns a [`ContextDefinition`] and a [`PoolConfig`] into a kube
//! [`Config`] (TLS flags and SOCKS5 handling from E03-S07 hook in here), and
//! [`build_client`] turns that into a [`Client`] under E03-S04's
//! [`ExecInteractivePolicy`] and error classification. [`ClientFactory`] is the
//! seam the pool calls, so tests can count, slow down or fail builds.
//!
//! Both steps block: `build_config` reads certificate files and `build_client` runs
//! exec credential plugins synchronously (kube 4.2 `Auth::try_from`). The pool
//! calls the factory on `tokio::task::spawn_blocking` under
//! [`PoolConfig::exec_deadline`], which is why the factory itself is synchronous.

use kube::config::{KubeConfigOptions, KubeconfigError};
use kube::{Client, Config};
use oxikube_domain::OxiError;

use super::config::PoolConfig;
use super::entry::ContextDefinition;
use crate::auth::{CredentialRefresh, ExecInteractivePolicy, classify_with};

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
        build_client(kube_config, definition, config.exec_policy)
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
/// An unparseable proxy URL, whether from the cluster's `proxy-url`, from
/// `proxy_env`, or from the process `HTTPS_PROXY` that kube reads itself, is a
/// [`Validation`](oxikube_domain::ErrorKind::Validation) error naming those
/// settings; the URL itself is never echoed (it may carry credentials).
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
        Some(url) => Some(url.parse().map_err(|_| invalid_proxy(context.as_str()))?),
        None => None,
    };
    Ok(config)
}

/// Builds the client under `policy`. Runs exec credential plugins, so it blocks.
///
/// The policy is E03-S04's [`ExecInteractivePolicy`]: a plugin that requires
/// interaction beyond it is rejected before it runs. Failures are classified by
/// [`classify_with`](crate::auth::classify_with) (scrubbed, credential-free
/// messages) with one exception: client certificate and key loading errors
/// (`kube::Error::InferKubeconfig`) go through the same mapping as
/// [`build_config`]'s kubeconfig errors, which never quotes kube's text, because
/// a base64 decode error names an offending byte of `client-key-data`.
pub fn build_client(
    mut config: Config,
    definition: &ContextDefinition,
    policy: ExecInteractivePolicy,
) -> Result<Client, OxiError> {
    policy.apply_to_config(&mut config)?;
    let refresh = CredentialRefresh::of(&config.auth_info);
    Client::try_from(config).map_err(|err| match err {
        kube::Error::InferKubeconfig(err) => kubeconfig_error(definition.context().as_str(), err),
        err => classify_with(&err, refresh),
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

/// The proxy error. Never quotes the URL: it may carry `user:password@`.
fn invalid_proxy(context: &str) -> OxiError {
    OxiError::validation(format!(
        "context `{context}`: the proxy URL (kubeconfig `proxy-url`, or `HTTPS_PROXY` / \
         `https_proxy` in the environment) is not a valid URI"
    ))
}

/// Maps kube's kubeconfig errors.
///
/// The source is attached only for variants whose messages are known to carry no
/// credential material (names, paths, URL parse errors). Certificate and key
/// loading errors are not among them: a base64 decode error quotes an offending
/// byte and its offset, which for `client-key-data` is part of the private key,
/// and `OxiError`'s `Debug` prints the source text.
fn kubeconfig_error(context: &str, err: KubeconfigError) -> OxiError {
    let (base, safe_source) = match &err {
        KubeconfigError::LoadContext(_) => (
            OxiError::not_found(format!("context `{context}` has no definition")),
            true,
        ),
        KubeconfigError::ParseProxyUrl(_) => (invalid_proxy(context), false),
        KubeconfigError::LoadClientKey(_) => (
            OxiError::validation(format!(
                "context `{context}`: the client key could not be loaded"
            )),
            false,
        ),
        KubeconfigError::LoadClientCertificate(_) => (
            OxiError::validation(format!(
                "context `{context}`: the client certificate could not be loaded"
            )),
            false,
        ),
        KubeconfigError::LoadCertificateAuthority(_) | KubeconfigError::ParseCertificates(_) => (
            OxiError::validation(format!(
                "context `{context}`: the certificate authority could not be loaded"
            )),
            false,
        ),
        KubeconfigError::CurrentContextNotSet
        | KubeconfigError::KindMismatch
        | KubeconfigError::ApiVersionMismatch
        | KubeconfigError::LoadClusterOfContext(_)
        | KubeconfigError::FindPath
        | KubeconfigError::ReadConfig(..)
        | KubeconfigError::MissingClusterUrl
        | KubeconfigError::ParseClusterUrl(_) => (
            OxiError::validation(format!("context `{context}`: invalid kubeconfig entry")),
            true,
        ),
        // `Parse` can quote YAML (credentials included); unknown future variants
        // get the same conservative treatment.
        _ => (
            OxiError::validation(format!("context `{context}`: invalid kubeconfig entry")),
            false,
        ),
    };
    if safe_source {
        base.with_source(err)
    } else {
        base
    }
}
