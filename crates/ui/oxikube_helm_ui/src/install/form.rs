//! [`InstallForm`]: what the person typed, what is wrong with it, and the spec it makes.

use std::time::Duration;

use oxikube_app::InstallSpec;
use oxikube_domain::helm::{ChartRef, ValuesYaml, validate_namespace_name, validate_release_name};
use oxikube_domain::ids::ClusterId;

/// The longest `--timeout` the form accepts, in seconds (a day).
const MAX_TIMEOUT_SECS: u64 = 86_400;

/// The form's fields, as plain data (the values are the editor's).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallForm {
    /// The release name.
    pub release: String,
    /// The namespace.
    pub namespace: String,
    /// Create the namespace when it is missing.
    pub create_namespace: bool,
    /// The chart version; empty is the newest.
    pub version: String,
    /// Wait for the resources to become ready.
    pub wait: bool,
    /// Helm's `--timeout` in seconds; empty is Helm's default.
    pub timeout: String,
}

/// What blocks the preview, in the person's words; `None` where the field is fine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormErrors {
    /// About the release name.
    pub release: Option<String>,
    /// About the namespace.
    pub namespace: Option<String>,
    /// About the timeout.
    pub timeout: Option<String>,
}

impl FormErrors {
    /// Nothing blocks the preview.
    pub fn is_empty(&self) -> bool {
        self.release.is_none() && self.namespace.is_none() && self.timeout.is_none()
    }
}

impl InstallForm {
    /// What is wrong, given the cluster's namespaces (`None` while they are unknown). A namespace
    /// that does not exist needs "Create namespace".
    pub fn errors(&self, namespaces: Option<&[String]>) -> FormErrors {
        let release = self.release.trim();
        let namespace = self.namespace.trim();
        let missing = namespaces.is_some_and(|known| !known.iter().any(|n| n == namespace));
        FormErrors {
            release: validate_release_name(release).err(),
            namespace: validate_namespace_name(namespace).err().or_else(|| {
                (missing && !self.create_namespace).then(|| {
                    format!("Namespace {namespace} does not exist: tick \"Create namespace\".")
                })
            }),
            timeout: self.timeout_value().err(),
        }
    }

    /// Whether the namespace typed is one the cluster does not have.
    pub fn namespace_is_new(&self, namespaces: Option<&[String]>) -> bool {
        let namespace = self.namespace.trim();
        namespaces
            .is_some_and(|known| !namespace.is_empty() && !known.iter().any(|n| n == namespace))
    }

    fn timeout_value(&self) -> Result<Option<Duration>, String> {
        let text = self.timeout.trim().trim_end_matches('s');
        if text.is_empty() {
            return Ok(None);
        }
        match text.parse::<u64>() {
            Ok(secs) if (1..=MAX_TIMEOUT_SECS).contains(&secs) => {
                Ok(Some(Duration::from_secs(secs)))
            }
            _ => Err(format!("A number of seconds from 1 to {MAX_TIMEOUT_SECS}.")),
        }
    }

    /// The spec installing `chart` (its version replaced by the form's) on `cluster` with
    /// `values`.
    ///
    /// # Errors
    ///
    /// What blocks it (see [`errors`](Self::errors)).
    pub fn spec(
        &self,
        cluster: &ClusterId,
        chart: &ChartRef,
        values: ValuesYaml,
        namespaces: Option<&[String]>,
    ) -> Result<InstallSpec, FormErrors> {
        let errors = self.errors(namespaces);
        if !errors.is_empty() {
            return Err(errors);
        }
        let mut chart = chart.clone();
        self.version.trim().clone_into(&mut chart.version);
        let mut spec = InstallSpec::new(
            cluster.clone(),
            self.release.trim(),
            self.namespace.trim(),
            chart,
        );
        spec.create_namespace = self.create_namespace;
        spec.values = values;
        spec.wait = self.wait;
        spec.timeout = self.timeout_value().ok().flatten();
        Ok(spec)
    }
}
