//! The in-cluster fallback: a service account as a synthetic kubeconfig context.
//!
//! Inside a pod there is no kubeconfig; the credentials are the mounted service account and the
//! API server comes from `KUBERNETES_SERVICE_HOST` / `KUBERNETES_SERVICE_PORT`. Instead of
//! carrying a separate `kube::Config` next to the catalog, the fallback is expressed as one
//! ordinary [`Kubeconfig`] context named [`IN_CLUSTER_CONTEXT`], so the source list and the
//! client pool treat it like any other entry. It mirrors what `kube::Config::incluster()` builds
//! (same server URL rule, CA file, `token-file` credential and default namespace) and, like it,
//! never reads the token: the user entry holds only the token file's *path*, which kube re-reads
//! when the projected token rotates. Nothing here can leak the token.
//!
//! The synthetic context has a fixed source label ([`IN_CLUSTER_SOURCE_LABEL`]) for its
//! [`ClusterId`], and the source path [`IN_CLUSTER_SOURCE_PATH`], which is not a real file (it
//! must not be watched).
//!
//! # Gaps against `kube::Config::incluster()`, and the fix-up
//!
//! Building a client from a `Kubeconfig` (`Config::from_custom_kubeconfig`) differs from
//! `Config::incluster()` in two ways, which [`apply_in_cluster_fixups`] closes. The client pool
//! (E03-S03) must call it on the `kube::Config` it builds for [`IN_CLUSTER_CONTEXT`]:
//!
//! * `root_cert_file` is set only by `incluster()`. It makes kube watch the CA file and reload
//!   it when the cluster CA rotates; from a kubeconfig it is `None`.
//! * `proxy_url` is read from the cluster entry or `HTTPS_PROXY` / `https_proxy` for
//!   kubeconfigs, while `incluster()` never uses a proxy. The fix-up clears it, a deliberate
//!   match with `incluster()`: the pod's own API server is reached directly.
//!
//! Token refresh needs no fix-up: the `token-file` credential is re-read like `incluster()`'s.
//!
//! Manual check (cannot run without a cluster): in a pod, load with [`Env::from_process`] and
//! build a client from the `in-cluster` context.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use kube::config::{
    AuthInfo, Cluster, Context, Kubeconfig, NamedAuthInfo, NamedCluster, NamedContext,
};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::{OxiError, OxiResult};

use super::env::Env;

/// Name of the synthetic context (and of its cluster and user entries).
pub const IN_CLUSTER_CONTEXT: &str = "in-cluster";
/// Fixed kubeconfig-source label hashed into the in-cluster [`ClusterId`].
pub const IN_CLUSTER_SOURCE_LABEL: &str = "in-cluster";
/// The in-cluster source's `SourceInfo::path`: not a real file, never watch it.
pub const IN_CLUSTER_SOURCE_PATH: &str = "<in-cluster>";

/// Mounted service account token (a credential: only the path is ever used here).
pub const SERVICE_ACCOUNT_TOKEN_FILE: &str = "/var/run/secrets/kubernetes.io/serviceaccount/token";
/// Mounted cluster CA bundle.
pub const SERVICE_ACCOUNT_CA_FILE: &str = "/var/run/secrets/kubernetes.io/serviceaccount/ca.crt";
/// Mounted namespace of the pod.
pub const SERVICE_ACCOUNT_NAMESPACE_FILE: &str =
    "/var/run/secrets/kubernetes.io/serviceaccount/namespace";

/// The in-cluster context name.
pub fn in_cluster_context_name() -> ContextName {
    ContextName::new(IN_CLUSTER_CONTEXT)
}

/// The catalog id of the in-cluster context: a fixed label plus the context name.
pub fn in_cluster_cluster_id() -> ClusterId {
    ClusterId::new(IN_CLUSTER_SOURCE_LABEL, &in_cluster_context_name())
}

/// The API server URL from the service host and port, as `kube::Config::incluster()` forms it:
/// `https://host[:port]`, port omitted when 443, IPv6 addresses bracketed. `None` when the host
/// is empty or the port is not a valid non-zero `u16`.
pub fn in_cluster_server_url(host: &str, port: &str) -> Option<String> {
    let port: u16 = port.trim().parse().ok().filter(|p| *p != 0)?;
    let host = host.trim();
    if host.is_empty() {
        return None;
    }
    Some(match host.parse::<IpAddr>() {
        Ok(ip) if port == 443 => match ip {
            IpAddr::V4(ip) => format!("https://{ip}"),
            IpAddr::V6(ip) => format!("https://[{ip}]"),
        },
        Ok(ip) => format!("https://{}", SocketAddr::new(ip, port)),
        Err(_) if port == 443 => format!("https://{host}"),
        Err(_) => format!("https://{host}:{port}"),
    })
}

/// Make a client config built for the in-cluster context behave like `Config::incluster()`.
///
/// Does nothing for any other context. For [`IN_CLUSTER_CONTEXT`] it sets `root_cert_file` to
/// the mounted CA (so kube reloads a rotated CA) and clears `proxy_url`. See the module docs.
pub fn apply_in_cluster_fixups(context: &ContextName, config: &mut kube::Config) {
    if context.as_str() == IN_CLUSTER_CONTEXT {
        config.root_cert_file = Some(PathBuf::from(SERVICE_ACCOUNT_CA_FILE));
        config.proxy_url = None;
    }
}

/// Build the synthetic in-cluster kubeconfig from `env` (pure; no I/O).
///
/// One cluster, one user and one context, all named [`IN_CLUSTER_CONTEXT`], which is also the
/// current context. Fails with `NotFound` when `env` does not describe a pod
/// ([`Env::in_cluster_detected`]).
pub fn in_cluster_kubeconfig(env: &Env) -> OxiResult<Kubeconfig> {
    let server = match (&env.kubernetes_service_host, &env.kubernetes_service_port) {
        (Some(host), Some(port)) if env.service_account_mounted => {
            in_cluster_server_url(host, port)
        }
        _ => None,
    }
    .ok_or_else(|| OxiError::not_found("not running in a cluster (no service account found)"))?;
    let name = IN_CLUSTER_CONTEXT.to_owned();
    Ok(Kubeconfig {
        clusters: vec![NamedCluster {
            name: name.clone(),
            cluster: Some(Cluster {
                server: Some(server),
                certificate_authority: Some(SERVICE_ACCOUNT_CA_FILE.to_owned()),
                ..Cluster::default()
            }),
            ..NamedCluster::default()
        }],
        auth_infos: vec![NamedAuthInfo {
            name: name.clone(),
            auth_info: Some(AuthInfo {
                token_file: Some(SERVICE_ACCOUNT_TOKEN_FILE.to_owned()),
                ..AuthInfo::default()
            }),
            ..NamedAuthInfo::default()
        }],
        contexts: vec![NamedContext {
            name: name.clone(),
            context: Some(Context {
                cluster: name.clone(),
                user: Some(name.clone()),
                namespace: env.service_account_namespace.clone(),
                ..Context::default()
            }),
            ..NamedContext::default()
        }],
        current_context: Some(name),
        ..Kubeconfig::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_url_follows_kube_incluster() {
        let cases = [
            ("fake.io", "8080", Some("https://fake.io:8080")),
            (
                "kubernetes.default.svc",
                "443",
                Some("https://kubernetes.default.svc"),
            ),
            ("10.11.12.13", "6443", Some("https://10.11.12.13:6443")),
            ("10.11.12.13", "443", Some("https://10.11.12.13")),
            ("2001:db8::1", "6443", Some("https://[2001:db8::1]:6443")),
            ("2001:db8::1", "443", Some("https://[2001:db8::1]")),
            ("", "443", None),
            ("10.0.0.1", "", None),
            ("10.0.0.1", "0", None),
            ("10.0.0.1", "70000", None),
            ("10.0.0.1", "https", None),
        ];
        for (host, port, expected) in cases {
            assert_eq!(
                in_cluster_server_url(host, port).as_deref(),
                expected,
                "{host}:{port}"
            );
        }
    }

    fn client_config() -> kube::Config {
        kube::Config::new("https://10.0.0.1".parse().unwrap())
    }

    #[test]
    fn fixups_restore_ca_reload_and_drop_the_proxy_for_in_cluster() {
        let mut config = client_config();
        config.proxy_url = Some("http://proxy.example:3128".parse().unwrap());
        assert!(config.root_cert_file.is_none());

        apply_in_cluster_fixups(&in_cluster_context_name(), &mut config);

        assert_eq!(
            config.root_cert_file.as_deref(),
            Some(std::path::Path::new(SERVICE_ACCOUNT_CA_FILE))
        );
        assert!(config.proxy_url.is_none());
    }

    #[test]
    fn fixups_leave_other_contexts_alone() {
        let mut config = client_config();
        config.proxy_url = Some("http://proxy.example:3128".parse().unwrap());

        apply_in_cluster_fixups(&"prod".into(), &mut config);

        assert!(config.root_cert_file.is_none());
        assert!(config.proxy_url.is_some());
    }

    #[test]
    fn kubeconfig_requires_a_pod_environment() {
        let err = in_cluster_kubeconfig(&Env::default()).unwrap_err();
        assert_eq!(err.kind(), oxikube_domain::ErrorKind::NotFound);
    }

    #[test]
    fn context_name_and_id_are_fixed() {
        assert_eq!(in_cluster_context_name().as_str(), "in-cluster");
        assert_eq!(in_cluster_cluster_id(), in_cluster_cluster_id());
        assert_ne!(
            in_cluster_cluster_id(),
            ClusterId::new("/home/u/.kube/config", &in_cluster_context_name())
        );
    }
}
