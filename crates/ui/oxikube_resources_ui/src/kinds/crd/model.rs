//! [`PanelModel`]: the CRD panel's text, built once per change of the CRD so drawing allocates
//! nothing. Pure: tested without a window.

use std::sync::Arc;

use gpui::SharedString;
use oxikube_domain::view::{ConditionStatus, CrdConversion, CrdSummary, CrdVersionSummary};

/// What a flag chip means; the panel picks the theme colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagTone {
    /// Plain.
    Neutral,
    /// Good (`storage`).
    Ok,
    /// Needs a look (`deprecated`, `not served`).
    Warn,
}

/// One printer column of a version, as drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnLine {
    /// The column's header.
    pub name: SharedString,
    /// `Name  type  .json.path`, with `(wide, priority N)` for a wide one.
    pub text: SharedString,
    /// Hidden by default (`priority > 0`).
    pub wide: bool,
}

/// One version, as drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionModel {
    /// The version.
    pub name: SharedString,
    /// Its flags, in a fixed order: not served, storage, deprecated, no schema, status, scale.
    pub flags: Vec<(&'static str, FlagTone)>,
    /// The deprecation warning, when the CRD sets one.
    pub warning: Option<SharedString>,
    /// The printer columns, in the CRD's order.
    pub columns: Vec<ColumnLine>,
}

/// Everything the panel draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelModel {
    /// `Established`.
    pub established: ConditionStatus,
    /// `NamesAccepted`.
    pub names_accepted: ConditionStatus,
    /// The facts above the versions, as (key, value); empty values are left out.
    pub facts: Vec<(&'static str, SharedString)>,
    /// The versions, in the CRD's order.
    pub versions: Vec<VersionModel>,
    /// Whether a version is served, so the custom resources can be opened.
    pub can_open: bool,
}

impl PanelModel {
    /// The model of `crd`.
    pub fn new(crd: &CrdSummary) -> Self {
        Self {
            established: crd.established(),
            names_accepted: crd.names_accepted(),
            facts: facts(crd),
            versions: crd.versions.iter().map(version).collect(),
            can_open: crd.display_version().is_some(),
        }
    }

    /// The value of the fact `key`, if it is shown.
    pub fn fact(&self, key: &str) -> Option<&str> {
        self.facts
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.as_ref())
    }
}

fn join(list: &[Arc<str>]) -> String {
    list.iter().map(|s| &**s).collect::<Vec<_>>().join(", ")
}

fn facts(crd: &CrdSummary) -> Vec<(&'static str, SharedString)> {
    let conversion = match &crd.conversion {
        CrdConversion::None => "None".to_owned(),
        CrdConversion::Webhook { service, url, .. } => {
            match service.as_deref().or(url.as_deref()) {
                Some(target) => format!("Webhook ({target})"),
                None => "Webhook".to_owned(),
            }
        }
    };
    let scope = if crd.scope.is_namespaced() {
        "Namespaced"
    } else {
        "Cluster"
    };
    [
        ("Kind", crd.names.kind.to_string()),
        ("Group", crd.group.to_string()),
        ("Scope", scope.to_owned()),
        ("Plural", crd.names.plural.to_string()),
        (
            "Singular",
            crd.names.singular.as_deref().unwrap_or("").to_owned(),
        ),
        (
            "List kind",
            crd.names.list_kind.as_deref().unwrap_or("").to_owned(),
        ),
        ("Short names", join(&crd.names.short_names)),
        ("Categories", join(&crd.names.categories)),
        ("Conversion", conversion),
        ("Stored versions", join(&crd.stored_versions)),
    ]
    .into_iter()
    .filter(|(_, value)| !value.is_empty())
    .map(|(key, value)| (key, value.into()))
    .collect()
}

fn version(v: &CrdVersionSummary) -> VersionModel {
    let flags = [
        (!v.served).then_some(("not served", FlagTone::Warn)),
        v.storage.then_some(("storage", FlagTone::Ok)),
        v.deprecated.then_some(("deprecated", FlagTone::Warn)),
        (!v.has_schema).then_some(("no schema", FlagTone::Neutral)),
        v.status_subresource
            .then_some(("status", FlagTone::Neutral)),
        v.scale_subresource.then_some(("scale", FlagTone::Neutral)),
    ];
    VersionModel {
        name: v.name.to_string().into(),
        flags: flags.into_iter().flatten().collect(),
        warning: v
            .deprecation_warning
            .as_deref()
            .map(|w| w.to_owned().into()),
        columns: v
            .printer_columns
            .iter()
            .map(|c| {
                let mut text = format!("{}  {}  {}", c.name, c.column_type, c.json_path);
                if c.is_wide() {
                    text.push_str(&format!("  (wide, priority {})", c.priority));
                }
                ColumnLine {
                    name: c.name.to_string().into(),
                    text: text.into(),
                    wide: c.is_wide(),
                }
            })
            .collect(),
    }
}
