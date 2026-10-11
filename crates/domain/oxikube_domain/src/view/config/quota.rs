//! [`QuotaSummary`]: a ResourceQuota's `used` against its `hard` limits.

use std::collections::BTreeSet;
use std::sync::Arc;

use super::super::{ViewError, check_kind, str_of, sub};
use super::util::{quantities, strings};
use crate::quantity::Quantity;
use crate::resource::Resource;

/// How full a quota line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuotaLevel {
    /// Below 80 % of the limit (or no limit to compare with).
    Ok,
    /// At least 80 % of the limit.
    Warn,
    /// At the limit or over it.
    Full,
}

/// One resource of a quota: what is used, what is allowed, and the ratio.
#[derive(Debug, Clone, PartialEq)]
pub struct QuotaLine {
    /// The resource name: `pods`, `requests.cpu`, `count/deployments.apps`,
    /// `requests.nvidia.com/gpu`.
    pub resource: Arc<str>,
    /// `status.used` as the API spelled it; `None` when the quota has not counted it yet.
    pub used: Option<Arc<str>>,
    /// `status.hard` (else `spec.hard`) as the API spelled it.
    pub hard: Option<Arc<str>>,
    /// `used / hard` (`0.25` for a quarter), `None` when either side is missing or unparseable;
    /// infinite when the limit is zero and something is used. Display only.
    pub ratio: Option<f64>,
}

impl QuotaLine {
    /// Whether this line counts objects (`count/...` and the plain object counts such as
    /// `pods` and `services`) rather than measuring a resource.
    pub fn is_count(&self) -> bool {
        const COUNTED: &[&str] = &[
            "pods",
            "services",
            "services.loadbalancers",
            "services.nodeports",
            "secrets",
            "configmaps",
            "persistentvolumeclaims",
            "replicationcontrollers",
            "resourcequotas",
        ];
        // `gold.storageclass.storage.k8s.io/persistentvolumeclaims` counts claims of one class.
        self.resource.starts_with("count/")
            || self.resource.ends_with("/persistentvolumeclaims")
            || COUNTED.contains(&&*self.resource)
    }

    /// The ratio as a percentage, rounded down (`25`); `None` without one.
    pub fn percent(&self) -> Option<u32> {
        self.ratio.map(|r| {
            if r.is_finite() {
                (r * 100.0).clamp(0.0, 100_000.0) as u32
            } else {
                100_000
            }
        })
    }

    /// How full the line is.
    pub fn level(&self) -> QuotaLevel {
        match self.ratio {
            Some(r) if r >= 1.0 => QuotaLevel::Full,
            Some(r) if r >= 0.8 => QuotaLevel::Warn,
            _ => QuotaLevel::Ok,
        }
    }

    /// `used / hard` as text: `1500m / 4`, `9 / 10`; `-` for a missing side.
    pub fn text(&self) -> String {
        format!(
            "{} / {}",
            self.used.as_deref().unwrap_or("-"),
            self.hard.as_deref().unwrap_or("-")
        )
    }
}

/// A ResourceQuota: one [`QuotaLine`] per resource, in name order.
#[derive(Debug, Clone, PartialEq)]
pub struct QuotaSummary {
    /// The lines.
    pub lines: Vec<QuotaLine>,
    /// `spec.scopes` (`NotTerminating`, `BestEffort`, ...).
    pub scopes: Vec<Arc<str>>,
    /// `spec.scopeSelector` match expressions as `scope op (values)`.
    pub scope_selector: Vec<String>,
}

impl QuotaSummary {
    /// The summary of a core `ResourceQuota`. Limits come from `status.hard` (what the quota
    /// controller enforces), else `spec.hard`.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not a `ResourceQuota`.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(res, "ResourceQuota", &[("", "ResourceQuota")])?;
        let (spec, status) = (sub(res.json(), "spec"), sub(res.json(), "status"));
        let mut hard = quantities(status, "hard");
        if hard.is_empty() {
            hard = quantities(spec, "hard");
        }
        let used = quantities(status, "used");
        let names: BTreeSet<&Arc<str>> = hard.keys().chain(used.keys()).collect();
        let lines = names
            .into_iter()
            .map(|name| {
                let (u, h) = (used.get(name).cloned(), hard.get(name).cloned());
                let ratio = ratio_of(u.as_deref(), h.as_deref());
                QuotaLine {
                    resource: name.clone(),
                    used: u,
                    hard: h,
                    ratio,
                }
            })
            .collect();
        let scope_selector = sub(spec, "scopeSelector")
            .get("matchExpressions")
            .and_then(|m| m.as_array())
            .map(|exprs| {
                exprs
                    .iter()
                    .map(|e| {
                        let scope = str_of(e, "scopeName").unwrap_or("?");
                        let op = str_of(e, "operator").unwrap_or("?");
                        let values = strings(e, "values").join(",");
                        if values.is_empty() {
                            format!("{scope} {op}")
                        } else {
                            format!("{scope} {op} ({values})")
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            lines,
            scopes: strings(spec, "scopes"),
            scope_selector,
        })
    }
}

/// `used / hard` as an exact quantity ratio.
fn ratio_of(used: Option<&str>, hard: Option<&str>) -> Option<f64> {
    let used = Quantity::parse(used?).ok()?;
    let hard = Quantity::parse(hard?).ok()?;
    match used.percent_of(&hard) {
        Some(percent) => Some(percent / 100.0),
        None if used.is_zero() => Some(0.0),
        None => Some(f64::INFINITY),
    }
}
