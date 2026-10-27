//! [`PriorityClassSummary`] and [`RuntimeClassSummary`].

use std::sync::Arc;

use super::super::{ViewError, arc_of, arr_of, bool_of, check_kind, i64_of, str_of, sub};
use super::util::quantities;
use crate::json::JsonRef;
use crate::resource::Resource;

/// A PriorityClass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorityClassSummary {
    /// `value`.
    pub value: i64,
    /// Whether pods without a class get this one (`globalDefault`).
    pub global_default: bool,
    /// `preemptionPolicy`; the API default is `PreemptLowerPriority`.
    pub preemption_policy: Arc<str>,
    /// `description`.
    pub description: Option<Arc<str>>,
}

impl PriorityClassSummary {
    /// The summary of a `PriorityClass`.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not a PriorityClass.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(
            res,
            "PriorityClass",
            &[("scheduling.k8s.io", "PriorityClass")],
        )?;
        let json = res.json();
        Ok(Self {
            value: i64_of(json, "value").unwrap_or(0),
            global_default: bool_of(json, "globalDefault"),
            preemption_policy: arc_of(json, "preemptionPolicy")
                .unwrap_or_else(|| Arc::from("PreemptLowerPriority")),
            description: arc_of(json, "description"),
        })
    }
}

/// A RuntimeClass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeClassSummary {
    /// `handler`: the CRI handler (`runsc`).
    pub handler: Arc<str>,
    /// `overhead.podFixed`: resources added to every pod of the class.
    pub overhead: Vec<(Arc<str>, Arc<str>)>,
    /// `scheduling.nodeSelector`.
    pub node_selector: Vec<(Arc<str>, Arc<str>)>,
    /// `scheduling.tolerations` as `key=value:effect` text.
    pub tolerations: Vec<String>,
}

impl RuntimeClassSummary {
    /// The summary of a `RuntimeClass`.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not a RuntimeClass.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(res, "RuntimeClass", &[("node.k8s.io", "RuntimeClass")])?;
        let json = res.json();
        let scheduling = sub(json, "scheduling");
        Ok(Self {
            handler: arc_of(json, "handler").unwrap_or_else(|| Arc::from("")),
            overhead: quantities(sub(json, "overhead"), "podFixed")
                .into_iter()
                .collect(),
            node_selector: quantities(scheduling, "nodeSelector").into_iter().collect(),
            tolerations: arr_of(scheduling, "tolerations")
                .iter()
                .map(toleration_text)
                .collect(),
        })
    }
}

/// `key=value:effect`, `key:effect` (`Exists`), or `op` alone for an empty toleration.
fn toleration_text(t: JsonRef<'_>) -> String {
    let key = str_of(t, "key").unwrap_or("");
    let mut out = String::from(if key.is_empty() { "<all>" } else { key });
    if let (Some("Equal") | None, Some(value)) = (str_of(t, "operator"), str_of(t, "value")) {
        out.push('=');
        out.push_str(value);
    }
    if let Some(effect) = str_of(t, "effect") {
        out.push(':');
        out.push_str(effect);
    }
    out
}
