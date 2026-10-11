//! The `prometheus` per-cluster preferences (E13-S05): [`PrometheusOverride`] and what it needs.

use oxikube_domain::MetricsProviderKind;

use crate::promql::{PromAuth, PromEndpoint, PromSecret};
use crate::secrets::SecretKey;
use crate::service_proxy::ServiceRef;

/// How a cluster's Prometheus is found (`prometheus.provider`, E13-S05).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum MetricsMode {
    /// Probe the cluster for known Prometheus-compatible services on connect.
    #[default]
    Auto,
    /// Use the endpoint the settings give (`url` or `path`), whatever the cluster has.
    Manual,
    /// Fetch no history metrics for the cluster (also what `enabled: false` means).
    Off,
}

impl MetricsMode {
    /// The settings spelling: `auto`, `manual` or `off`.
    pub const fn id(self) -> &'static str {
        match self {
            MetricsMode::Auto => "auto",
            MetricsMode::Manual => "manual",
            MetricsMode::Off => "off",
        }
    }
}

/// How requests to a configured Prometheus authenticate (`prometheus.auth`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum PrometheusAuthKind {
    /// No `Authorization` header.
    #[default]
    None,
    /// `Authorization: Bearer <the keychain entry>`.
    Bearer,
    /// `Authorization: Basic base64(auth_user:<the keychain entry>)`.
    Basic,
}

/// Where a cluster's Prometheus is and whether to use it (E13-S05).
///
/// Resolved from `clusters.<id>.prometheus` over the top-level block over the defaults, field by
/// field. The defaults are on, auto-detect, no auth; a cluster without a `prometheus` block has
/// [`ClusterPrefs::prometheus`](super::ClusterPrefs::prometheus) `None`, which reads the same. Only the names of keychain entries
/// are here, never a token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrometheusOverride {
    /// `false` fetches no history metrics for the cluster, whatever the mode says.
    pub enabled: bool,
    /// Auto-detect, configured by hand, or off.
    pub mode: MetricsMode,
    /// The stack a configured endpoint belongs to, so the query catalogue picks the right label
    /// names; `None` is a plain Prometheus. Detection classifies by itself and ignores it.
    pub kind: Option<MetricsProviderKind>,
    /// `namespace/service:port` reached through the API server's service proxy (`manual`).
    pub path: Option<String>,
    /// A direct `http(s)` URL, used instead of `path` (`manual`; no credentials, query or
    /// fragment).
    pub url: Option<String>,
    /// A path inserted before `/api/v1/...` (Mimir's `/prometheus`, VictoriaMetrics'
    /// `/select/0/prometheus`).
    pub path_prefix: Option<String>,
    /// How requests to a `url` authenticate. A `path` (service proxy) carries the connection's
    /// own credentials instead.
    pub auth: PrometheusAuthKind,
    /// The user name of basic auth (not secret).
    pub auth_user: Option<String>,
    /// The keychain entry holding the bearer token or the basic password.
    pub auth_secret: Option<SecretKey>,
}

impl Default for PrometheusOverride {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: MetricsMode::Auto,
            kind: None,
            path: None,
            url: None,
            path_prefix: None,
            auth: PrometheusAuthKind::None,
            auth_user: None,
            auth_secret: None,
        }
    }
}

impl PrometheusOverride {
    /// What the cluster does about history metrics: [`MetricsMode::Off`] when disabled, else the
    /// configured mode.
    pub fn effective_mode(&self) -> MetricsMode {
        if self.enabled {
            self.mode
        } else {
            MetricsMode::Off
        }
    }

    /// The endpoint a [`MetricsMode::Manual`] cluster queries, and the stack it belongs to.
    ///
    /// # Errors
    ///
    /// A sentence saying what is wrong with the settings, to show next to the cluster's
    /// metrics state: neither `url` nor `path` is set, `path` is malformed, the auth kind lacks
    /// its secret or user, or auth is combined with a service `path` (the service proxy carries
    /// the connection's own credentials, so a token has nowhere to go).
    pub fn manual_endpoint(&self) -> Result<(PromEndpoint, MetricsProviderKind), String> {
        let kind = self.kind.unwrap_or(MetricsProviderKind::Prometheus);
        let mut endpoint = if let Some(url) = &self.url {
            let mut endpoint = PromEndpoint::new(url.clone());
            endpoint.auth = self.prom_auth()?;
            endpoint
        } else if let Some(path) = &self.path {
            let service = ServiceRef::parse(path).ok_or_else(|| {
                "`prometheus.path` must look like namespace/service:port, for example \
                 monitoring/prometheus-operated:9090"
                    .to_owned()
            })?;
            if self.auth != PrometheusAuthKind::None {
                return Err(
                    "`prometheus.auth` applies to a `url`; a service `path` goes through the \
                     API server with your cluster credentials"
                        .to_owned(),
                );
            }
            PromEndpoint::service(service)
        } else {
            return Err(
                "manual mode needs a `prometheus.url` or a `prometheus.path` (namespace/service:port)"
                    .to_owned(),
            );
        };
        endpoint.path_prefix = self.path_prefix.clone();
        Ok((endpoint, kind))
    }

    fn prom_auth(&self) -> Result<PromAuth, String> {
        let secret = || {
            self.auth_secret
                .clone()
                .map(PromSecret::Keychain)
                .ok_or_else(|| "`prometheus.auth_secret` must name a keychain entry".to_owned())
        };
        match self.auth {
            PrometheusAuthKind::None => Ok(PromAuth::None),
            PrometheusAuthKind::Bearer => Ok(PromAuth::Bearer(secret()?)),
            PrometheusAuthKind::Basic => {
                let username = self
                    .auth_user
                    .clone()
                    .ok_or_else(|| "basic auth needs `prometheus.auth_user`".to_owned())?;
                Ok(PromAuth::Basic {
                    username,
                    password: secret()?,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manual(update: impl FnOnce(&mut PrometheusOverride)) -> PrometheusOverride {
        let mut prefs = PrometheusOverride {
            mode: MetricsMode::Manual,
            ..PrometheusOverride::default()
        };
        update(&mut prefs);
        prefs
    }

    #[test]
    fn a_disabled_cluster_is_off_whatever_the_mode() {
        let mut prefs = manual(|p| p.url = Some("https://prom".into()));
        assert_eq!(prefs.effective_mode(), MetricsMode::Manual);
        prefs.enabled = false;
        assert_eq!(prefs.effective_mode(), MetricsMode::Off);
        assert_eq!(
            PrometheusOverride::default().effective_mode(),
            MetricsMode::Auto
        );
    }

    #[test]
    fn a_manual_url_carries_its_prefix_kind_and_keychain_auth() {
        let prefs = manual(|p| {
            p.url = Some("https://vm.example.com".into());
            p.path_prefix = Some("/select/0/prometheus".into());
            p.kind = Some(MetricsProviderKind::VictoriaMetrics);
            p.auth = PrometheusAuthKind::Bearer;
            p.auth_secret = Some(SecretKey::new("prometheus", "vm").unwrap());
        });
        let (endpoint, kind) = prefs.manual_endpoint().unwrap();
        assert_eq!(kind, MetricsProviderKind::VictoriaMetrics);
        assert_eq!(endpoint.base_url, "https://vm.example.com");
        assert_eq!(
            endpoint.path_prefix.as_deref(),
            Some("/select/0/prometheus")
        );
        assert!(matches!(
            endpoint.auth,
            PromAuth::Bearer(PromSecret::Keychain(ref key)) if key.to_string() == "prometheus/vm"
        ));
    }

    #[test]
    fn a_manual_service_path_goes_through_the_proxy() {
        let prefs = manual(|p| p.path = Some("monitoring/vmsingle-vmks:8429".into()));
        let (endpoint, kind) = prefs.manual_endpoint().unwrap();
        assert_eq!(kind, MetricsProviderKind::Prometheus);
        assert_eq!(
            endpoint.service.unwrap().short(),
            "monitoring/vmsingle-vmks:8429"
        );
    }

    #[test]
    fn a_url_wins_over_a_path() {
        let prefs = manual(|p| {
            p.url = Some("https://prom".into());
            p.path = Some("monitoring/prom:9090".into());
        });
        assert_eq!(prefs.manual_endpoint().unwrap().0.service, None);
    }

    #[test]
    fn misconfigurations_are_sentences_not_silence() {
        let none = manual(|_| {});
        assert!(none.manual_endpoint().unwrap_err().contains("url"));
        let bad_path = manual(|p| p.path = Some("prometheus".into()));
        assert!(
            bad_path
                .manual_endpoint()
                .unwrap_err()
                .contains("namespace/service:port")
        );
        let no_secret = manual(|p| {
            p.url = Some("https://prom".into());
            p.auth = PrometheusAuthKind::Bearer;
        });
        assert!(
            no_secret
                .manual_endpoint()
                .unwrap_err()
                .contains("auth_secret")
        );
        let no_user = manual(|p| {
            p.url = Some("https://prom".into());
            p.auth = PrometheusAuthKind::Basic;
            p.auth_secret = Some(SecretKey::new("prometheus", "x").unwrap());
        });
        assert!(no_user.manual_endpoint().unwrap_err().contains("auth_user"));
        let proxied_auth = manual(|p| {
            p.path = Some("monitoring/prom:9090".into());
            p.auth = PrometheusAuthKind::Bearer;
            p.auth_secret = Some(SecretKey::new("prometheus", "x").unwrap());
        });
        assert!(
            proxied_auth
                .manual_endpoint()
                .unwrap_err()
                .contains("cluster credentials")
        );
    }
}
