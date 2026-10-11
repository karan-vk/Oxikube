//! View-models of the Config section's smaller kinds (E12-S12): quotas, limit ranges,
//! autoscalers, disruption budgets, priority and runtime classes, leases and the admission kinds.
//!
//! Every type is built from a [`Resource`](crate::Resource) with `from_resource`, which fails
//! only on the wrong kind ([`ViewError::WrongKind`](super::ViewError)). Missing or wrongly typed
//! fields degrade to defaults. The table columns, the detail panels and the agent summaries read
//! these, so the text of a cell and the text of a panel cannot drift apart.
//!
//! | File | Holds |
//! |---|---|
//! | `quota` | [`QuotaSummary`]: `used` against `hard` per resource, with ratios (`count/pods`, extended resources) |
//! | `limits` | [`LimitRangeSummary`]: min / max / default / default request / ratio per type and resource |
//! | `hpa` | [`HpaSummary`], [`HpaMetric`]: `autoscaling/v2` metrics with targets and current values (v1 CPU fields read as one metric), the scale target |
//! | `vpa` | [`VpaSummary`]: update mode, conditions, recommendations and container policies |
//! | `pdb` | [`PdbSummary`]: budget, healthy counts, allowed disruptions |
//! | `lease` | [`LeaseSummary`]: holder, duration, acquire and renew times |
//! | `classes` | [`PriorityClassSummary`], [`RuntimeClassSummary`] |
//! | `webhook` | [`WebhookConfigSummary`]: the webhooks of a mutating or validating configuration (the `caBundle` is reported as present or absent, never kept) |
//! | `admission` | [`AdmissionPolicySummary`], [`AdmissionBindingSummary`] |
//! | `util` | [`TargetRef`], selector text and the small readers they share |
//!
//! # Performance
//!
//! Same contract as the other view-models: one walk over the JSON per object, allocating only
//! the output strings. These kinds are small tables; a constructor runs once per delta per
//! visible row, and the detail panels cache the result per resource version.

mod admission;
mod classes;
mod hpa;
mod lease;
mod limits;
mod pdb;
mod quota;
mod util;
mod vpa;
mod webhook;

#[cfg(test)]
mod tests;

pub use admission::{AdmissionBindingSummary, AdmissionPolicySummary, ParamRef, PolicyValidation};
pub use classes::{PriorityClassSummary, RuntimeClassSummary};
pub use hpa::{HpaMetric, HpaMetricKind, HpaState, HpaSummary};
pub use lease::LeaseSummary;
pub use limits::{LimitRangeSummary, LimitRow, LimitSection};
pub use pdb::PdbSummary;
pub use quota::{QuotaLevel, QuotaLine, QuotaSummary};
pub use util::{TargetRef, selector_text};
pub use vpa::{VpaContainerPolicy, VpaRecommendation, VpaSummary};
pub use webhook::{
    CaBundle, WebhookConfigSummary, WebhookKind, WebhookRule, WebhookSummary, WebhookTarget,
};
