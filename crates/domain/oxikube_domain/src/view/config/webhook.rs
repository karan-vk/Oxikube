//! [`WebhookConfigSummary`]: the webhooks of a Mutating or ValidatingWebhookConfiguration.
//!
//! The `caBundle` is a certificate blob; the summary records only whether one is set
//! ([`CaBundle`]) and never keeps the text.

use std::sync::Arc;

use super::super::{ViewError, arc_of, arr_of, check_kind, i64_of, str_of, sub};
use super::util::{selector_text, strings};
use crate::json::JsonRef;
use crate::resource::Resource;

/// Which configuration kind a summary describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WebhookKind {
    /// `MutatingWebhookConfiguration`.
    Mutating,
    /// `ValidatingWebhookConfiguration`.
    Validating,
}

/// Where a webhook is called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookTarget {
    /// A Service in the cluster.
    Service {
        /// Service namespace.
        namespace: Arc<str>,
        /// Service name.
        name: Arc<str>,
        /// URL path, when set.
        path: Option<Arc<str>>,
        /// Port, when set (the API default is 443).
        port: Option<i64>,
    },
    /// An absolute URL.
    Url(Arc<str>),
    /// Neither is set.
    Unset,
}

impl WebhookTarget {
    /// `namespace/name:port/path` for a service, the URL for a URL, `-` for none.
    pub fn text(&self) -> String {
        match self {
            Self::Service {
                namespace,
                name,
                path,
                port,
            } => format!(
                "{namespace}/{name}:{}{}",
                port.unwrap_or(443),
                path.as_deref().unwrap_or("")
            ),
            Self::Url(url) => url.to_string(),
            Self::Unset => "-".to_owned(),
        }
    }
}

/// Whether `clientConfig.caBundle` is set. The bytes are never kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaBundle {
    /// A bundle is set.
    Present,
    /// No bundle: the API server trusts the system roots.
    Absent,
}

impl CaBundle {
    /// `Present` or `Absent`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Present => "Present",
            Self::Absent => "Absent",
        }
    }
}

/// One rule of a webhook or policy: which operations on which resources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookRule {
    /// `operations` (`CREATE`, `UPDATE`, `DELETE`, `CONNECT`, `*`).
    pub operations: Vec<Arc<str>>,
    /// `apiGroups`.
    pub api_groups: Vec<Arc<str>>,
    /// `apiVersions`.
    pub api_versions: Vec<Arc<str>>,
    /// `resources`.
    pub resources: Vec<Arc<str>>,
    /// `scope`: `Namespaced`, `Cluster` or `*`.
    pub scope: Option<Arc<str>>,
}

impl WebhookRule {
    /// Reads a `rules` / `resourceRules` entry.
    pub(super) fn read(v: JsonRef<'_>) -> Self {
        Self {
            operations: strings(v, "operations"),
            api_groups: strings(v, "apiGroups"),
            api_versions: strings(v, "apiVersions"),
            resources: strings(v, "resources"),
            scope: arc_of(v, "scope"),
        }
    }
}

/// One webhook of a configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookSummary {
    /// `name`.
    pub name: Arc<str>,
    /// Where it is called.
    pub target: WebhookTarget,
    /// Whether a `caBundle` is set.
    pub ca_bundle: CaBundle,
    /// `failurePolicy`; the v1 default is `Fail`.
    pub failure_policy: Arc<str>,
    /// `sideEffects`.
    pub side_effects: Option<Arc<str>>,
    /// `matchPolicy`; the v1 default is `Equivalent`.
    pub match_policy: Arc<str>,
    /// `timeoutSeconds`; the default is 10.
    pub timeout_seconds: i64,
    /// `reinvocationPolicy` (mutating webhooks only).
    pub reinvocation_policy: Option<Arc<str>>,
    /// `admissionReviewVersions`.
    pub review_versions: Vec<Arc<str>>,
    /// `namespaceSelector` as text.
    pub namespace_selector: Option<String>,
    /// `objectSelector` as text.
    pub object_selector: Option<String>,
    /// `rules`.
    pub rules: Vec<WebhookRule>,
    /// How many `matchConditions` it has.
    pub match_conditions: usize,
}

/// A Mutating or ValidatingWebhookConfiguration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookConfigSummary {
    /// Which kind.
    pub kind: WebhookKind,
    /// `webhooks`, in order.
    pub webhooks: Vec<WebhookSummary>,
}

impl WebhookConfigSummary {
    /// The summary of a Mutating or ValidatingWebhookConfiguration.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is neither.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        const GROUP: &str = "admissionregistration.k8s.io";
        check_kind(
            res,
            "MutatingWebhookConfiguration or ValidatingWebhookConfiguration",
            &[
                (GROUP, "MutatingWebhookConfiguration"),
                (GROUP, "ValidatingWebhookConfiguration"),
            ],
        )?;
        let kind = if &*res.kind.kind == "MutatingWebhookConfiguration" {
            WebhookKind::Mutating
        } else {
            WebhookKind::Validating
        };
        Ok(Self {
            kind,
            webhooks: arr_of(res.json(), "webhooks")
                .iter()
                .map(read_webhook)
                .collect(),
        })
    }
}

fn read_webhook(w: JsonRef<'_>) -> WebhookSummary {
    let client = sub(w, "clientConfig");
    let target = if let Some(service) = client.get("service") {
        WebhookTarget::Service {
            namespace: arc_of(service, "namespace").unwrap_or_else(|| Arc::from("")),
            name: arc_of(service, "name").unwrap_or_else(|| Arc::from("")),
            path: arc_of(service, "path"),
            port: i64_of(service, "port"),
        }
    } else {
        str_of(client, "url").map_or(WebhookTarget::Unset, |u| WebhookTarget::Url(Arc::from(u)))
    };
    WebhookSummary {
        name: arc_of(w, "name").unwrap_or_else(|| Arc::from("")),
        target,
        ca_bundle: if str_of(client, "caBundle").is_some() {
            CaBundle::Present
        } else {
            CaBundle::Absent
        },
        failure_policy: arc_of(w, "failurePolicy").unwrap_or_else(|| Arc::from("Fail")),
        side_effects: arc_of(w, "sideEffects"),
        match_policy: arc_of(w, "matchPolicy").unwrap_or_else(|| Arc::from("Equivalent")),
        timeout_seconds: i64_of(w, "timeoutSeconds").unwrap_or(10),
        reinvocation_policy: arc_of(w, "reinvocationPolicy"),
        review_versions: strings(w, "admissionReviewVersions"),
        namespace_selector: w.get("namespaceSelector").and_then(selector_text),
        object_selector: w.get("objectSelector").and_then(selector_text),
        rules: arr_of(w, "rules").iter().map(WebhookRule::read).collect(),
        match_conditions: arr_of(w, "matchConditions").len(),
    }
}
