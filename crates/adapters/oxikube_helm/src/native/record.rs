//! Helm's release JSON (private serde shapes) and its mapping onto the domain types.
//!
//! Two shapes are read from the same JSON: a [`Header`] for list and history rows (chart,
//! status, times, description) and a full [`Record`] (plus values, manifest, notes) for the
//! detail methods. Unknown fields are ignored, so newer Helm versions keep decoding.
//!
//! The header shape names only the fields a row shows: `chart.values` (the chart's defaults,
//! thousands of keys for the large charts) and `info.notes` are skipped by the parser without
//! being built, so a list pays for the metadata, not for the record's bulk.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::helm::{ChartRef, Release, ReleaseRef, ReleaseStatus, Revision, Values};
use serde::Deserialize;
use serde_json::Value;

use super::storage::StoredRevision;

/// The cheap part of a release record: what a list or history row shows.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Header {
    /// `chart.metadata.name` and `.version`.
    pub chart: ChartRef,
    /// `chart.metadata.appVersion`, when set.
    pub app_version: Option<String>,
    /// `info.status`, when present.
    pub status: Option<ReleaseStatus>,
    /// `info.last_deployed`, when it parses.
    pub updated: Option<Timestamp>,
    /// `info.description`.
    pub description: String,
}

/// A fully decoded revision. Holds values and a manifest that may embed credentials: kept
/// in memory only, never logged (`Values` redacts its `Debug`).
#[derive(Clone)]
pub(crate) struct Record {
    /// The list fields.
    pub header: Header,
    /// `info.notes` (`NOTES.txt` as rendered), `None` when empty.
    pub notes: Option<Arc<str>>,
    /// `config`: the values the user supplied.
    pub user_values: Values,
    /// `chart.values`: the chart's defaults, when the record kept them.
    pub chart_values: Option<Values>,
    /// `manifest`: the rendered YAML.
    pub manifest: Arc<str>,
}

impl Record {
    /// The chart defaults merged with the user's values (Helm coalescing: maps merge, lists
    /// replace, `null` deletes). Best effort: without chart defaults in the record this is
    /// just the user's values.
    pub fn computed_values(&self) -> Values {
        match &self.chart_values {
            Some(defaults) => Values::merge(defaults, &self.user_values),
            None => Values::merge(&Values::default(), &self.user_values),
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct RawHeader {
    #[serde(default)]
    info: RawHeaderInfo,
    #[serde(default)]
    chart: Option<RawHeaderChart>,
}

/// `info` without `notes`.
#[derive(Deserialize, Default)]
struct RawHeaderInfo {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    last_deployed: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

/// `chart` without `values` (or templates, files, schema).
#[derive(Deserialize, Default)]
struct RawHeaderChart {
    #[serde(default)]
    metadata: Option<RawMetadata>,
}

#[derive(Deserialize)]
pub(crate) struct RawRecord {
    #[serde(default)]
    info: RawInfo,
    #[serde(default)]
    chart: Option<RawChart>,
    #[serde(default)]
    config: Option<Value>,
    #[serde(default)]
    manifest: Option<String>,
}

/// `info`: the header's fields plus `notes`.
#[derive(Deserialize, Default)]
struct RawInfo {
    #[serde(flatten)]
    header: RawHeaderInfo,
    #[serde(default)]
    notes: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawChart {
    #[serde(default)]
    metadata: Option<RawMetadata>,
    #[serde(default)]
    values: Option<Value>,
}

#[derive(Deserialize, Default)]
struct RawMetadata {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default, rename = "appVersion")]
    app_version: Option<String>,
}

impl RawHeader {
    pub fn into_header(self) -> Header {
        let metadata = self.chart.and_then(|c| c.metadata);
        header(self.info, metadata.as_ref())
    }
}

impl RawRecord {
    pub fn into_record(self) -> Record {
        let (metadata, chart_values) = self
            .chart
            .map_or((None, None), |chart| (chart.metadata, chart.values));
        let notes = self
            .info
            .notes
            .as_deref()
            .filter(|n| !n.trim().is_empty())
            .map(Arc::from);
        Record {
            notes,
            user_values: self.config.map(Values::from_json).unwrap_or_default(),
            // Moved, not copied: the chart's defaults can be thousands of keys.
            chart_values: chart_values.map(Values::from_json),
            manifest: Arc::from(self.manifest.unwrap_or_default()),
            header: header(self.info.header, metadata.as_ref()),
        }
    }
}

fn header(info: RawHeaderInfo, metadata: Option<&RawMetadata>) -> Header {
    let field = |f: fn(&RawMetadata) -> &Option<String>| {
        metadata.and_then(|m| f(m).clone()).unwrap_or_default()
    };
    Header {
        chart: ChartRef::new(field(|m| &m.name), field(|m| &m.version)),
        app_version: metadata
            .and_then(|m| m.app_version.clone())
            .filter(|v| !v.is_empty()),
        status: info.status.as_deref().map(ReleaseStatus::parse),
        updated: info
            .last_deployed
            .as_deref()
            .and_then(|t| t.parse::<Timestamp>().ok()),
        description: info.description.unwrap_or_default(),
    }
}

/// The list row of `stored`, from its decoded header, or from labels alone when the header
/// could not be read (`None`): then the chart is blank but the release still shows.
pub(crate) fn release_row(
    reference: ReleaseRef,
    stored: &StoredRevision,
    header: Option<&Header>,
) -> Release {
    let chart = header.map_or_else(|| ChartRef::new("", ""), |h| h.chart.clone());
    let mut release = Release::summary(
        reference,
        chart,
        stored.revision,
        status(stored, header),
        updated(stored, header),
    );
    release.app_version = header.and_then(|h| h.app_version.clone());
    release
}

/// The history entry of `stored`.
pub(crate) fn revision_row(stored: &StoredRevision, header: Option<&Header>) -> Revision {
    Revision {
        revision: stored.revision,
        status: status(stored, header),
        chart: header.map_or_else(|| ChartRef::new("", ""), |h| h.chart.clone()),
        app_version: header.and_then(|h| h.app_version.clone()),
        updated: updated(stored, header),
        description: header.map(|h| h.description.clone()).unwrap_or_default(),
    }
}

/// The payload's status is authoritative; the label is the fallback.
fn status(stored: &StoredRevision, header: Option<&Header>) -> ReleaseStatus {
    header
        .and_then(|h| h.status)
        .filter(|s| *s != ReleaseStatus::Unknown)
        .unwrap_or(stored.status)
}

fn updated(stored: &StoredRevision, header: Option<&Header>) -> Timestamp {
    header
        .and_then(|h| h.updated)
        .or(stored.modified)
        .unwrap_or(Timestamp::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_header_never_builds_the_chart_values_or_the_notes() {
        // `1e400` overflows an f64: building a JSON tree of it fails, skipping it does not. So the
        // header decoding below only succeeds when `chart.values` and `info.notes` are skipped.
        let text = r#"{
            "info": {"status": "deployed", "description": "Upgrade complete",
                     "notes": {"not": "a string", "n": 1e400}},
            "chart": {"metadata": {"name": "web", "version": "1.2.3", "appVersion": "2.0"},
                      "values": {"replicas": 1e400, "deep": [{"a": 1e400}]}}
        }"#;
        assert!(serde_json::from_str::<RawRecord>(text).is_err());
        let header = serde_json::from_str::<RawHeader>(text)
            .expect("header")
            .into_header();
        assert_eq!(header.chart, ChartRef::new("web", "1.2.3"));
        assert_eq!(header.app_version.as_deref(), Some("2.0"));
        assert_eq!(header.status, Some(ReleaseStatus::Deployed));
        assert_eq!(header.description, "Upgrade complete");
    }

    #[test]
    fn the_record_keeps_the_header_fields() {
        let record = serde_json::from_value::<RawRecord>(json!({
            "info": {"status": "failed", "description": "boom", "notes": "hi"},
            "chart": {"metadata": {"name": "web", "version": "1"}, "values": {"a": 1}},
            "config": {"b": 2},
            "manifest": "---",
        }))
        .unwrap()
        .into_record();
        assert_eq!(record.header.chart, ChartRef::new("web", "1"));
        assert_eq!(record.header.status, Some(ReleaseStatus::Failed));
        assert_eq!(record.header.description, "boom");
        assert_eq!(record.notes.as_deref(), Some("hi"));
        assert!(record.chart_values.is_some());
    }
}
