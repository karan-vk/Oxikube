//! [`CrdSummary`]: a CustomResourceDefinition as the CRD list and its detail read it (E12-S17).
//!
//! The list's computed columns (`Versions`, `Aliases`, `Established`) and the detail panel
//! (names, versions with their printer columns, conversion, conditions) read the same answers
//! from here. [`version_order`] is Kubernetes' version priority, shared by everything that picks
//! "the newest served version".

use std::sync::Arc;

use jiff::Timestamp;

mod cells;

pub use cells::{crd_aliases_text, crd_versions_text, version_order};

use super::{
    Condition, ConditionStatus, ViewError, arc_of, arr_of, bool_of, check_kind, i64_of, str_of, sub,
};
use crate::ids::Scope;
use crate::json::JsonRef;
use crate::resource::Resource;

/// `spec.names` of a CRD.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CrdNames {
    /// The kind (`Widget`).
    pub kind: Arc<str>,
    /// The plural resource name (`widgets`): the path segment and what `kubectl get` takes.
    pub plural: Arc<str>,
    /// The singular name (`widget`), when set.
    pub singular: Option<Arc<str>>,
    /// The list kind (`WidgetList`), when set.
    pub list_kind: Option<Arc<str>>,
    /// `shortNames` (`wd`), in the CRD's order.
    pub short_names: Vec<Arc<str>>,
    /// `categories` (`all`): the groups of kinds `kubectl get <category>` lists.
    pub categories: Vec<Arc<str>>,
}

/// One `additionalPrinterColumns` entry of a version: a column `kubectl get` shows for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrinterColumn {
    /// The header (`Replicas`).
    pub name: Arc<str>,
    /// The OpenAPI type (`integer`, `string`, `date`, ...).
    pub column_type: Arc<str>,
    /// The OpenAPI format, when set.
    pub format: Option<Arc<str>>,
    /// The JSONPath the server evaluates (`.spec.replicas`).
    pub json_path: Arc<str>,
    /// `0` shows by default; above zero only with `-o wide` (and hidden by default here).
    pub priority: i64,
    /// The column's description, when set.
    pub description: Option<Arc<str>>,
}

impl PrinterColumn {
    /// Whether `kubectl get` hides it unless `-o wide` (`priority > 0`).
    pub fn is_wide(&self) -> bool {
        self.priority > 0
    }
}

/// One `spec.versions` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrdVersionSummary {
    /// The version (`v1beta1`).
    pub name: Arc<str>,
    /// Whether the API server serves it.
    pub served: bool,
    /// Whether objects are persisted at it (exactly one version is).
    pub storage: bool,
    /// Whether the CRD marks it deprecated.
    pub deprecated: bool,
    /// The warning the server returns to clients of a deprecated version, when set.
    pub deprecation_warning: Option<Arc<str>>,
    /// Whether it declares an `openAPIV3Schema`.
    pub has_schema: bool,
    /// Whether it enables the `status` subresource.
    pub status_subresource: bool,
    /// Whether it enables the `scale` subresource.
    pub scale_subresource: bool,
    /// `additionalPrinterColumns`, in the CRD's order.
    pub printer_columns: Vec<PrinterColumn>,
}

impl CrdVersionSummary {
    /// The printer columns `kubectl get` shows by default (`priority` 0), in order.
    pub fn default_columns(&self) -> impl Iterator<Item = &PrinterColumn> {
        self.printer_columns.iter().filter(|c| !c.is_wide())
    }

    /// The printer columns only `kubectl get -o wide` shows (`priority > 0`), in order.
    pub fn wide_columns(&self) -> impl Iterator<Item = &PrinterColumn> {
        self.printer_columns.iter().filter(|c| c.is_wide())
    }
}

/// `spec.conversion`: how the server converts objects between versions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CrdConversion {
    /// `None` (the default): only `apiVersion` changes between versions.
    #[default]
    None,
    /// A conversion webhook converts them.
    Webhook {
        /// The webhook's service as `namespace/name[:port][path]`, when it is a service.
        service: Option<Arc<str>>,
        /// The webhook's URL, when it is one.
        url: Option<Arc<str>>,
        /// `conversionReviewVersions`.
        review_versions: Vec<Arc<str>>,
    },
}

impl CrdConversion {
    /// `None` or `Webhook`, as the API spells the strategy.
    pub fn strategy(&self) -> &'static str {
        match self {
            CrdConversion::None => "None",
            CrdConversion::Webhook { .. } => "Webhook",
        }
    }
}

/// A CustomResourceDefinition, typed: what the CRD list and its detail show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrdSummary {
    /// `metadata.name` (`widgets.example.com`).
    pub name: Arc<str>,
    /// `spec.group`.
    pub group: Arc<str>,
    /// Namespaced or cluster-scoped custom resources.
    pub scope: Scope,
    /// `spec.names`.
    pub names: CrdNames,
    /// `spec.versions`, in the CRD's order.
    pub versions: Vec<CrdVersionSummary>,
    /// `spec.conversion`.
    pub conversion: CrdConversion,
    /// `status.conditions` (`Established`, `NamesAccepted`, ...), in API order.
    pub conditions: Vec<Condition>,
    /// `status.storedVersions`: every version objects were ever stored at.
    pub stored_versions: Vec<Arc<str>>,
    /// `metadata.creationTimestamp`.
    pub created: Option<Timestamp>,
}

impl CrdSummary {
    /// Builds the summary of a CRD.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not an `apiextensions.k8s.io`
    /// `CustomResourceDefinition`. Missing fields read as empty.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(
            res,
            "CustomResourceDefinition",
            &[("apiextensions.k8s.io", "CustomResourceDefinition")],
        )?;
        let json = res.json();
        let spec = sub(json, "spec");
        let status = sub(json, "status");
        Ok(Self {
            name: res.meta.name.clone(),
            group: arc_of(spec, "group").unwrap_or_else(|| Arc::from("")),
            scope: match str_of(spec, "scope") {
                Some("Cluster") => Scope::Cluster,
                _ => Scope::Namespaced,
            },
            names: names(sub(spec, "names")),
            versions: arr_of(spec, "versions")
                .iter()
                .filter_map(version)
                .collect(),
            conversion: conversion(sub(spec, "conversion")),
            conditions: arr_of(status, "conditions")
                .iter()
                .filter_map(Condition::from_json)
                .collect(),
            stored_versions: strings(status, "storedVersions"),
            created: res.meta.creation,
        })
    }

    /// The `Established` condition: whether the server serves the custom resources yet.
    pub fn established(&self) -> ConditionStatus {
        self.condition("Established")
    }

    /// The `NamesAccepted` condition: whether the names conflict with another CRD's.
    pub fn names_accepted(&self) -> ConditionStatus {
        self.condition("NamesAccepted")
    }

    fn condition(&self, kind: &str) -> ConditionStatus {
        self.conditions
            .iter()
            .rev()
            .find(|c| &*c.kind == kind)
            .map_or(ConditionStatus::Unknown, |c| c.status)
    }

    /// The version a list of the custom resources opens: the storage version when it is served,
    /// else the newest served one ([`version_order`]). `None` when nothing is served.
    pub fn display_version(&self) -> Option<&CrdVersionSummary> {
        self.versions
            .iter()
            .find(|v| v.storage && v.served)
            .or_else(|| {
                self.versions
                    .iter()
                    .filter(|v| v.served)
                    .min_by(|a, b| version_order(&a.name, &b.name))
            })
    }

    /// The version `name`, when the CRD declares it.
    pub fn version(&self, name: &str) -> Option<&CrdVersionSummary> {
        self.versions.iter().find(|v| &*v.name == name)
    }
}

fn strings(v: JsonRef<'_>, key: &str) -> Vec<Arc<str>> {
    arr_of(v, key)
        .iter()
        .filter_map(JsonRef::as_str)
        .filter(|s| !s.is_empty())
        .map(Arc::from)
        .collect()
}

fn names(n: JsonRef<'_>) -> CrdNames {
    CrdNames {
        kind: arc_of(n, "kind").unwrap_or_else(|| Arc::from("")),
        plural: arc_of(n, "plural").unwrap_or_else(|| Arc::from("")),
        singular: arc_of(n, "singular"),
        list_kind: arc_of(n, "listKind"),
        short_names: strings(n, "shortNames"),
        categories: strings(n, "categories"),
    }
}

fn version(v: JsonRef<'_>) -> Option<CrdVersionSummary> {
    let subresources = sub(v, "subresources");
    Some(CrdVersionSummary {
        name: arc_of(v, "name")?,
        served: bool_of(v, "served"),
        storage: bool_of(v, "storage"),
        deprecated: bool_of(v, "deprecated"),
        deprecation_warning: arc_of(v, "deprecationWarning"),
        has_schema: v.pointer("/schema/openAPIV3Schema").is_some(),
        status_subresource: subresources.get("status").is_some(),
        scale_subresource: subresources.get("scale").is_some(),
        printer_columns: arr_of(v, "additionalPrinterColumns")
            .iter()
            .filter_map(printer_column)
            .collect(),
    })
}

fn printer_column(c: JsonRef<'_>) -> Option<PrinterColumn> {
    Some(PrinterColumn {
        name: arc_of(c, "name")?,
        column_type: arc_of(c, "type").unwrap_or_else(|| Arc::from("string")),
        format: arc_of(c, "format"),
        json_path: arc_of(c, "jsonPath").unwrap_or_else(|| Arc::from("")),
        priority: i64_of(c, "priority").unwrap_or(0),
        description: arc_of(c, "description"),
    })
}

fn conversion(c: JsonRef<'_>) -> CrdConversion {
    if str_of(c, "strategy") != Some("Webhook") {
        return CrdConversion::None;
    }
    let webhook = sub(c, "webhook");
    let config = sub(webhook, "clientConfig");
    let service = config.get("service").filter(|s| s.is_object()).map(|s| {
        let mut text = format!(
            "{}/{}",
            str_of(s, "namespace").unwrap_or(""),
            str_of(s, "name").unwrap_or("")
        );
        if let Some(port) = i64_of(s, "port") {
            text.push_str(&format!(":{port}"));
        }
        if let Some(path) = str_of(s, "path") {
            text.push_str(path);
        }
        Arc::from(text)
    });
    CrdConversion::Webhook {
        service,
        url: arc_of(config, "url"),
        review_versions: strings(webhook, "conversionReviewVersions"),
    }
}
