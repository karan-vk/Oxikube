//! The `prometheus` block of the per-cluster settings (E13-S05): whether a cluster's history
//! metrics are fetched, where from, and how to authenticate.
//!
//! ```json
//! "prometheus": {
//!   "enabled": true,                       // false: fetch no history metrics for the cluster
//!   "provider": "auto",                    // auto (probe the cluster) | manual | off
//!   "kind": "victoria_metrics",            // manual: which stack, for the query variants
//!   "url": "https://vm.example.com",       // manual: a direct URL ...
//!   "path": "monitoring/vmsingle:8429",    // ... or namespace/service:port via the API server
//!   "path_prefix": "/select/0/prometheus", // inserted before /api/v1/...
//!   "auth": "bearer",                      // none | bearer | basic (a `url` only)
//!   "auth_user": "grafana",                // basic only
//!   "auth_secret": "prod-prometheus"       // the keychain entry holding the token or password
//! }
//! ```
//!
//! Fields merge across layers (defaults, the user's top-level block, `clusters.<id>`). Without
//! `provider`, a block with a `url` or `path` is `manual` and anything else `auto`. The file keeps
//! no secret: `auth_secret` only names an OS keychain entry (namespace `prometheus`).

use oxikube_domain::MetricsProviderKind;
use oxikube_ports::{
    MetricsMode, PrometheusAuthKind, PrometheusOverride, ServiceRef, secrets::SecretKey,
};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use super::fields::{HttpUrl, SecretName};
use super::resolved::non_blank;

/// How the cluster's Prometheus is found (`prometheus.provider`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderModeContent {
    /// Probe the cluster for known Prometheus-compatible services when it connects.
    Auto,
    /// Use the `url` or `path` below, whatever the cluster has.
    Manual,
    /// Fetch no history metrics for the cluster.
    Off,
}

/// How requests to a `url` authenticate (`prometheus.auth`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthContent {
    /// No `Authorization` header.
    None,
    /// `Authorization: Bearer` with the token in the `auth_secret` keychain entry.
    Bearer,
    /// HTTP basic auth: `auth_user` and the password in the `auth_secret` keychain entry.
    Basic,
}

/// `namespace/service:port` (a port number or name; `https:` before the service name selects
/// TLS), the Service the API server proxies to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ServicePath(String);

impl ServicePath {
    /// The text as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ServicePath {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let text = value.trim();
        if ServiceRef::parse(text).is_none() {
            return Err(
                "a service path must look like namespace/service:port, for example \
                        monitoring/prometheus-operated:9090"
                    .into(),
            );
        }
        Ok(Self(text.to_owned()))
    }
}

impl From<ServicePath> for String {
    fn from(value: ServicePath) -> Self {
        value.0
    }
}

impl JsonSchema for ServicePath {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ServicePath".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^[A-Za-z0-9.-]+/(https:)?[A-Za-z0-9.-]+:[A-Za-z0-9.-]+$",
            "description": "namespace/service:port, reached through the API server's service proxy.",
            "examples": ["monitoring/prometheus-operated:9090"]
        })
    }
}

/// A path inserted before `/api/v1/...`: segments of letters, digits and `-_.~`, no `..`, query
/// or fragment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PathPrefix(String);

impl PathPrefix {
    /// The prefix as written (trimmed).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PathPrefix {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let text = value.trim();
        let ok = text.split('/').all(|segment| {
            segment != ".."
                && segment != "."
                && segment
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~'))
        });
        if !ok {
            return Err("a path prefix is path segments such as /prometheus or \
                        /select/0/prometheus, with no query, fragment or `..`"
                .into());
        }
        Ok(Self(text.to_owned()))
    }
}

impl From<PathPrefix> for String {
    fn from(value: PathPrefix) -> Self {
        value.0
    }
}

impl JsonSchema for PathPrefix {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "PathPrefix".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^(/?[A-Za-z0-9_.~-]+)*/?$",
            "description": "A path inserted before /api/v1/..., for example /prometheus.",
            "examples": ["/prometheus", "/select/0/prometheus"]
        })
    }
}

/// Schema of `kind`: the ids of `MetricsProviderKind`.
fn kind_schema(_: &mut SchemaGenerator) -> Schema {
    let ids: Vec<&str> = MetricsProviderKind::ALL
        .iter()
        .map(|kind| kind.id())
        .collect();
    json_schema!({
        "type": ["string", "null"],
        "enum": ids.into_iter().map(Some).chain([None]).collect::<Vec<_>>(),
        "description": "The stack a manual endpoint belongs to, so history queries use its label names. Unset is a plain Prometheus."
    })
}

/// Whether and where a cluster's history metrics come from (the `prometheus` key): auto-detection
/// (the default), a manual `url` or service `path`, or off. Keeps no secret: the token lives in
/// the OS keychain and `auth_secret` only names its entry.
///
/// Example: `{ "provider": "manual", "url": "https://vm.example.com", "kind": "victoria_metrics",
/// "path_prefix": "/select/0/prometheus", "auth_secret": "prod-vm" }`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PrometheusContent {
    /// `false` fetches no history metrics for the cluster, whatever `provider` says. The live
    /// usage columns from metrics-server are not affected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// `auto` probes the cluster for a Prometheus-compatible service when it connects; `manual`
    /// uses the `url` or `path`; `off` fetches nothing. Unset means `manual` when a `url` or
    /// `path` is set and `auto` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderModeContent>,
    /// The stack a manual endpoint belongs to (`kube_prometheus`, `lens`, `victoria_metrics`,
    /// `mimir`, `openshift`, `prometheus`), so history queries use its label names. Unset is a
    /// plain Prometheus. Detection classifies the stack itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "kind_schema")]
    pub kind: Option<MetricsProviderKind>,
    /// `namespace/service:port`, reached through the API server's service proxy with your
    /// cluster credentials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<ServicePath>,
    /// A direct `http(s)` URL, used instead of `path`. No credentials, query or fragment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<HttpUrl>,
    /// A path inserted before `/api/v1/...`: Mimir's `/prometheus`, VictoriaMetrics' cluster
    /// `/select/0/prometheus`, a Grafana datasource proxy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<PathPrefix>,
    /// How requests to a `url` authenticate. Unset means `bearer` when `auth_secret` is set
    /// (`basic` when `auth_user` is too) and `none` otherwise. A service `path` needs no auth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthContent>,
    /// The user name of basic auth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_user: Option<String>,
    /// Name of the keychain entry (namespace `prometheus`) that holds the bearer token or the
    /// basic password. Never put the token itself in `settings.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_secret: Option<SecretName>,
}

impl PrometheusContent {
    /// The plain value the app layer reads, or `None` when the block says nothing.
    pub(super) fn into_prefs(self) -> Option<PrometheusOverride> {
        let auth_user = non_blank(self.auth_user);
        let auth_secret: Option<SecretKey> = self.auth_secret.map(|name| name.key());
        let manual = self.url.is_some() || self.path.is_some();
        let value = PrometheusOverride {
            enabled: self.enabled.unwrap_or(true),
            mode: match self.provider {
                Some(ProviderModeContent::Auto) => MetricsMode::Auto,
                Some(ProviderModeContent::Manual) => MetricsMode::Manual,
                Some(ProviderModeContent::Off) => MetricsMode::Off,
                None if manual => MetricsMode::Manual,
                None => MetricsMode::Auto,
            },
            kind: self.kind,
            path: self.path.map(|path| path.as_str().to_owned()),
            url: self.url.map(|url| url.as_str().to_owned()),
            path_prefix: self
                .path_prefix
                .map(|prefix| prefix.as_str().to_owned())
                .filter(|prefix| !prefix.is_empty()),
            auth: match self.auth {
                Some(AuthContent::None) => PrometheusAuthKind::None,
                Some(AuthContent::Bearer) => PrometheusAuthKind::Bearer,
                Some(AuthContent::Basic) => PrometheusAuthKind::Basic,
                None if auth_secret.is_some() && auth_user.is_some() => PrometheusAuthKind::Basic,
                None if auth_secret.is_some() => PrometheusAuthKind::Bearer,
                None => PrometheusAuthKind::None,
            },
            auth_user,
            auth_secret,
        };
        (value != PrometheusOverride::default()).then_some(value)
    }
}
