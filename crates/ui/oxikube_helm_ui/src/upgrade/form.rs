//! [`UpgradeForm`]: what the person typed and picked, what is wrong with it, and the spec it makes.

use std::time::Duration;

use oxikube_app::UpgradeSpec;
use oxikube_domain::helm::{ChartRef, ReleaseRef, ValuesYaml};

use super::backend::ChartSource;

/// The longest `--timeout` the form accepts, in seconds (a day).
const MAX_TIMEOUT_SECS: u64 = 86_400;

/// The form's fields, as plain data (the values are the editor's).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpgradeForm {
    /// The chart to upgrade to, as `helm` takes it: `repo/name`, a path or an OCI reference.
    pub chart: String,
    /// The chart version; empty is the newest.
    pub version: String,
    /// Keep the previous revision's values under the edited ones (`--reuse-values`).
    pub reuse_values: bool,
    /// Wait for the resources to become ready.
    pub wait: bool,
    /// Helm's `--timeout` in seconds; empty is Helm's default.
    pub timeout: String,
}

/// What blocks the preview, in the person's words; `None` where the field is fine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormErrors {
    /// About the chart.
    pub chart: Option<String>,
    /// About the timeout.
    pub timeout: Option<String>,
}

impl FormErrors {
    /// Nothing blocks the preview.
    pub fn is_empty(&self) -> bool {
        self.chart.is_none() && self.timeout.is_none()
    }
}

impl UpgradeForm {
    /// What is wrong.
    pub fn errors(&self) -> FormErrors {
        FormErrors {
            chart: self
                .chart
                .trim()
                .is_empty()
                .then(|| "Enter the chart to upgrade to (repo/name).".to_owned()),
            timeout: self.timeout_value().err(),
        }
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

    /// The chart the form names, at its version: the repository's chart when `chart` is one of
    /// `sources` (`bitnami/nginx`), else the reference as typed (a path, an OCI reference).
    pub fn chart_ref(&self, sources: &[ChartSource]) -> ChartRef {
        let typed = self.chart.trim();
        let mut chart = sources
            .iter()
            .find(|source| source.chart.reference() == typed)
            .map_or_else(|| ChartRef::new(typed, ""), |source| source.chart.clone());
        self.version.trim().clone_into(&mut chart.version);
        chart
    }

    /// The spec upgrading `release` with `values`.
    ///
    /// # Errors
    ///
    /// What blocks it (see [`errors`](Self::errors)).
    pub fn spec(
        &self,
        release: &ReleaseRef,
        sources: &[ChartSource],
        values: ValuesYaml,
    ) -> Result<UpgradeSpec, FormErrors> {
        let errors = self.errors();
        if !errors.is_empty() {
            return Err(errors);
        }
        let mut spec = UpgradeSpec::new(release.clone(), self.chart_ref(sources));
        spec.values = values;
        spec.reuse_values = self.reuse_values;
        spec.wait = self.wait;
        spec.timeout = self.timeout_value().ok().flatten();
        Ok(spec)
    }
}
