//! [`LimitRangeSummary`]: the limits and defaults a LimitRange applies, one table per type.

use std::collections::BTreeSet;
use std::sync::Arc;

use super::super::{ViewError, arr_of, check_kind, str_of, sub};
use super::util::quantities;
use crate::resource::Resource;

/// One resource's row of a [`LimitSection`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LimitRow {
    /// The resource: `cpu`, `memory`, `storage`, `ephemeral-storage`.
    pub resource: Arc<str>,
    /// `min`.
    pub min: Option<Arc<str>>,
    /// `max`.
    pub max: Option<Arc<str>>,
    /// `default`: the limit given to a container that sets none.
    pub default: Option<Arc<str>>,
    /// `defaultRequest`: the request given to a container that sets none.
    pub default_request: Option<Arc<str>>,
    /// `maxLimitRequestRatio`.
    pub max_ratio: Option<Arc<str>>,
}

/// The limits of one `type` (`Container`, `Pod`, `PersistentVolumeClaim`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitSection {
    /// The limited type.
    pub kind: Arc<str>,
    /// One row per resource the section mentions, in name order.
    pub rows: Vec<LimitRow>,
}

/// A LimitRange's limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitRangeSummary {
    /// One section per `spec.limits` entry, in order.
    pub sections: Vec<LimitSection>,
}

impl LimitRangeSummary {
    /// The summary of a core `LimitRange`.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not a `LimitRange`.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(res, "LimitRange", &[("", "LimitRange")])?;
        let sections = arr_of(sub(res.json(), "spec"), "limits")
            .iter()
            .map(|item| {
                let (min, max) = (quantities(item, "min"), quantities(item, "max"));
                let (default, request) = (
                    quantities(item, "default"),
                    quantities(item, "defaultRequest"),
                );
                let ratio = quantities(item, "maxLimitRequestRatio");
                let names: BTreeSet<&Arc<str>> = [&min, &max, &default, &request, &ratio]
                    .into_iter()
                    .flat_map(|m| m.keys())
                    .collect();
                let rows = names
                    .into_iter()
                    .map(|name| LimitRow {
                        resource: name.clone(),
                        min: min.get(name).cloned(),
                        max: max.get(name).cloned(),
                        default: default.get(name).cloned(),
                        default_request: request.get(name).cloned(),
                        max_ratio: ratio.get(name).cloned(),
                    })
                    .collect();
                LimitSection {
                    kind: Arc::from(str_of(item, "type").unwrap_or("Container")),
                    rows,
                }
            })
            .collect();
        Ok(Self { sections })
    }
}
