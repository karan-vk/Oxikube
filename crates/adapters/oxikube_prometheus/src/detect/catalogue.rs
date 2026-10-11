//! The catalogue of known Prometheus-compatible services: data, not code.
//!
//! The built-in catalogue is `catalogue.json` next to this file, embedded at compile time. A
//! stack that is missing is one more entry; no matching code changes. Tests use their own
//! catalogues.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use oxikube_domain::MetricsProviderKind;
use oxikube_ports::ProxyScheme;
use serde::Deserialize;

/// How the API server speaks to the pod behind a catalogued Service.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SchemeEntry {
    /// Plain HTTP.
    #[default]
    Http,
    /// TLS.
    Https,
}

impl From<SchemeEntry> for ProxyScheme {
    fn from(value: SchemeEntry) -> Self {
        match value {
            SchemeEntry::Http => ProxyScheme::Http,
            SchemeEntry::Https => ProxyScheme::Https,
        }
    }
}

/// One kind of Service to look for.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// The stack a match belongs to.
    pub kind: MetricsProviderKind,
    /// The exact Service name, or `None` when matching by `service_prefix`.
    #[serde(default)]
    pub service: Option<String>,
    /// A Service name prefix (`vmsingle` matches `vmsingle-vmks`).
    #[serde(default)]
    pub service_prefix: Option<String>,
    /// The namespaces to look in.
    pub namespaces: Vec<String>,
    /// Port names or numbers, most preferred first.
    pub ports: Vec<String>,
    /// Plain HTTP or TLS towards the pod.
    #[serde(default)]
    pub scheme: SchemeEntry,
    /// Paths inserted before `/api/v1/...`, tried in order. Empty means the root only.
    #[serde(default)]
    pub path_prefixes: Vec<String>,
}

impl Entry {
    /// Whether a Service called `name` is one this entry looks for.
    pub fn matches(&self, name: &str) -> bool {
        match (&self.service, &self.service_prefix) {
            (Some(exact), _) => exact == name,
            (None, Some(prefix)) => name.starts_with(prefix.as_str()),
            (None, None) => false,
        }
    }

    /// The name as shown for a probe that found nothing: the Service name, or `prefix*`.
    pub fn pattern(&self) -> String {
        match (&self.service, &self.service_prefix) {
            (Some(exact), _) => exact.clone(),
            (None, Some(prefix)) => format!("{prefix}*"),
            (None, None) => String::new(),
        }
    }

    /// The prefixes to try: the configured ones, else the root.
    pub fn prefixes(&self) -> Vec<Option<String>> {
        if self.path_prefixes.is_empty() {
            return vec![None];
        }
        self.path_prefixes
            .iter()
            .map(|prefix| (!prefix.is_empty()).then(|| prefix.clone()))
            .collect()
    }
}

/// The ordered list of entries to try.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalogue {
    #[serde(default, rename = "_comment")]
    _comment: Option<String>,
    /// The entries, in the order they are tried.
    pub entries: Vec<Entry>,
}

impl Catalogue {
    /// The built-in catalogue.
    pub fn builtin() -> &'static Catalogue {
        static BUILTIN: OnceLock<Catalogue> = OnceLock::new();
        BUILTIN.get_or_init(|| {
            Catalogue::parse(include_str!("catalogue.json"))
                .unwrap_or_else(|e| unreachable!("catalogue.json is checked by a test: {e}"))
        })
    }

    /// Reads a catalogue from JSON.
    ///
    /// # Errors
    ///
    /// The JSON error when the text does not describe a catalogue.
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Every namespace some entry looks in, sorted.
    pub fn namespaces(&self) -> BTreeSet<&str> {
        self.entries
            .iter()
            .flat_map(|entry| entry.namespaces.iter().map(String::as_str))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_catalogue_covers_every_stack_of_the_acceptance_criteria() {
        let catalogue = Catalogue::builtin();
        let has = |name: &str, namespace: &str| {
            catalogue
                .entries
                .iter()
                .any(|e| e.matches(name) && e.namespaces.iter().any(|n| n == namespace))
        };
        assert!(has("prometheus-operated", "monitoring"));
        assert!(has("kube-prometheus-stack-prometheus", "monitoring"));
        assert!(has("prometheus", "lens-metrics"));
        assert!(has("vmsingle-vmks", "monitoring"));
        assert!(has("vmselect-vmcluster", "victoria-metrics"));
        assert!(has("mimir-query-frontend", "observability"));
        assert!(has("thanos-querier", "openshift-monitoring"));
        for kind in MetricsProviderKind::ALL {
            if kind != MetricsProviderKind::Prometheus {
                assert!(
                    catalogue.entries.iter().any(|e| e.kind == kind),
                    "{kind} has no entry"
                );
            }
        }
    }

    #[test]
    fn every_entry_names_a_service_and_a_port() {
        for entry in &Catalogue::builtin().entries {
            assert!(
                entry.service.is_some() != entry.service_prefix.is_some(),
                "{entry:?}: exactly one of service and service_prefix"
            );
            assert!(
                !entry.namespaces.is_empty() && !entry.ports.is_empty(),
                "{entry:?}"
            );
        }
    }

    #[test]
    fn matching_is_exact_or_by_prefix() {
        let entries = &Catalogue::builtin().entries;
        let exact = entries
            .iter()
            .find(|e| e.service.as_deref() == Some("prometheus-operated"));
        let exact = exact.unwrap();
        assert!(exact.matches("prometheus-operated") && !exact.matches("prometheus-operated-2"));
        let prefix = entries
            .iter()
            .find(|e| e.service_prefix.as_deref() == Some("vmsingle"));
        let prefix = prefix.unwrap();
        assert!(prefix.matches("vmsingle-vmks") && !prefix.matches("svc-vmsingle"));
        assert_eq!(prefix.pattern(), "vmsingle*");
        assert_eq!(exact.prefixes(), vec![None]);
    }

    #[test]
    fn an_unknown_field_is_an_error_not_silently_dropped() {
        let bad = r#"{"entries":[{"kind":"mimir","service":"x","namespaces":["a"],"ports":["1"],"prots":[]}]}"#;
        assert!(Catalogue::parse(bad).is_err());
    }
}
