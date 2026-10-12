//! [`AllocationRow`]: one resource of a node or namespace, with its text and bar ready.

use std::sync::Arc;

use oxikube_domain::Quantity;
use oxikube_domain::utilisation::percent;

use super::AllocationKind;

/// What the capacity of a row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityKind {
    /// A node's `status.allocatable`.
    Allocatable,
    /// The tightest `ResourceQuota` of a namespace.
    Quota,
}

/// The live usage of one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Usage {
    /// The row has no usage to show (the pod count).
    NotApplicable,
    /// The first reading has not arrived.
    Pending,
    /// The reading.
    Value(Quantity),
    /// metrics-server does not measure this resource (ephemeral storage, GPUs).
    NotReported,
    /// There is no reading, and why: the whole `metrics unavailable: <reason>` text.
    Unavailable(Arc<str>),
}

/// How many of the pods counted set no request, and no limit, for a row's resource.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnsetCount {
    /// Pods with no request for the resource.
    pub requests: u32,
    /// Pods with no limit for the resource.
    pub limits: u32,
}

/// Which amount fills a bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarBasis {
    /// Requests against the capacity (a node's allocatable, a quota's `requests.*`).
    Requests,
    /// Limits against a quota that caps only limits.
    Limits,
}

/// The three-segment bar of a row, as fractions of its track (`0.0..=1.0`, clamped; the text says
/// the real figure).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarModel {
    /// What [`fill`](Self::fill) measures.
    pub basis: BarBasis,
    /// The filled share: requests (or limits, see [`basis`](Self::basis)).
    pub fill: f32,
    /// Where the limits end, when they are measured against the same capacity as the fill.
    pub limits_mark: Option<f32>,
    /// The live usage, when there is a reading.
    pub usage: Option<f32>,
    /// Whether the filled amount reaches or passes the capacity.
    pub full: bool,
}

/// One resource of a node or a namespace.
///
/// `capacity` is what the requests are measured against: a node's `status.allocatable`, or a
/// namespace's tightest `requests.*` quota; `None` when nothing caps it (a namespace without
/// quotas), in which case the row has numbers and no bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocationRow {
    /// What the row measures.
    pub kind: AllocationKind,
    /// What [`capacity`](Self::capacity) is.
    pub capacity_kind: CapacityKind,
    /// The cap on requests.
    pub capacity: Option<Quantity>,
    /// The cap on limits: allocatable for a node, the tightest `limits.*` quota for a namespace.
    pub limits_capacity: Option<Quantity>,
    /// The sum of the pods' requests (for pods: how many there are).
    pub requests: Quantity,
    /// The sum of the pods' limits; `None` for the pod count, which has none.
    pub limits: Option<Quantity>,
    /// The live usage.
    pub usage: Usage,
    /// The pods that set no request or limit for this resource.
    pub unset: UnsetCount,
}

impl AllocationRow {
    /// Requests as a percent of the capacity; `None` without one (or a zero one).
    pub fn requests_percent(&self) -> Option<f64> {
        percent(&self.requests, self.capacity.as_ref()?)
    }

    /// Limits as a percent of the limits capacity (above 100 is over-commitment).
    pub fn limits_percent(&self) -> Option<f64> {
        percent(self.limits.as_ref()?, self.limits_capacity.as_ref()?)
    }

    /// Usage as a percent of the capacity the bar uses (`usage_scale`).
    pub fn usage_percent(&self) -> Option<f64> {
        let Usage::Value(value) = &self.usage else {
            return None;
        };
        percent(value, self.usage_scale()?)
    }

    /// What usage is measured against: the capacity, else the limits capacity.
    fn usage_scale(&self) -> Option<&Quantity> {
        self.capacity.as_ref().or(self.limits_capacity.as_ref())
    }

    /// Whether the limits go past what they are measured against. Normal on a node (the
    /// scheduler books requests, not limits), so a view *notes* it and never colours it as an
    /// error.
    pub fn limits_overcommitted(&self) -> bool {
        matches!((&self.limits, &self.limits_capacity), (Some(l), Some(c)) if l > c)
    }

    /// Whether the requests need more than the capacity gives.
    pub fn requests_exceed_capacity(&self) -> bool {
        self.capacity.as_ref().is_some_and(|c| self.requests > *c)
    }

    /// `1000m of 4000m (25%)`, or just `1000m` without a capacity.
    pub fn requests_text(&self) -> String {
        self.amount_text(
            &self.requests,
            self.capacity.as_ref(),
            self.requests_percent(),
        )
    }

    /// [`requests_text`](Self::requests_text) without the capacity: `1000m (25%)`. What a view
    /// prints when the capacity is already in the row's header.
    pub fn requests_short(&self) -> String {
        self.short_text(&self.requests, self.requests_percent())
    }

    /// `1500m of 4000m (37%)`; `None` for a resource with no limits (the pod count).
    pub fn limits_text(&self) -> Option<String> {
        let limits = self.limits.as_ref()?;
        Some(self.amount_text(limits, self.limits_capacity.as_ref(), self.limits_percent()))
    }

    /// [`limits_text`](Self::limits_text) without the capacity: `1500m (37%)`.
    pub fn limits_short(&self) -> Option<String> {
        Some(self.short_text(self.limits.as_ref()?, self.limits_percent()))
    }

    /// The reading with its share: `1200m (30%)`; `None` unless there is a reading. Against a
    /// quota the share says what it is a share of (`39% of the requests quota`): a quota does
    /// not cap usage, so the cap that stands in for it is named.
    pub fn usage_short(&self) -> Option<String> {
        let Usage::Value(value) = &self.usage else {
            return None;
        };
        let amount = self.kind.format(value);
        Some(match (self.usage_percent(), self.usage_basis()) {
            (Some(p), Some(basis)) => format!("{amount} ({} of the {basis})", whole(p)),
            (Some(p), None) => format!("{amount} ({})", whole(p)),
            (None, _) => amount,
        })
    }

    /// What a share of usage is a share of, when that is not obvious: a namespace's usage is
    /// shown against its `requests.*` quota, else its `limits.*` quota.
    fn usage_basis(&self) -> Option<&'static str> {
        if self.capacity_kind != CapacityKind::Quota {
            return None;
        }
        match (&self.capacity, &self.limits_capacity) {
            (Some(_), _) => Some("requests quota"),
            (None, Some(_)) => Some("limits quota"),
            (None, None) => None,
        }
    }

    /// The capacity as text (`3920m`), `None` when nothing caps the resource.
    pub fn capacity_text(&self) -> Option<String> {
        let scale = self.capacity.as_ref().or(self.limits_capacity.as_ref())?;
        Some(self.kind.format(scale))
    }

    /// The usage line: the reading with its share, or the visible reason there is none. `None`
    /// for a resource usage does not apply to.
    pub fn usage_text(&self) -> Option<String> {
        Some(match &self.usage {
            Usage::NotApplicable => return None,
            Usage::Pending => "reading usage...".to_owned(),
            Usage::NotReported => "usage is not reported by metrics-server".to_owned(),
            Usage::Unavailable(reason) => reason.to_string(),
            Usage::Value(value) => match self.usage_basis() {
                Some(_) => self.usage_short().unwrap_or_default(),
                None => self.amount_text(value, self.usage_scale(), self.usage_percent()),
            },
        })
    }

    /// `3 pods set no request, 5 set no limit`; `None` when every pod sets both.
    pub fn unset_text(&self) -> Option<String> {
        let UnsetCount { requests, limits } = self.unset;
        let pods = |n: u32| if n == 1 { "pod" } else { "pods" };
        match (requests, limits) {
            (0, 0) => None,
            (r, 0) => Some(format!("{r} {} set no request", pods(r))),
            (0, l) => Some(format!("{l} {} set no limit", pods(l))),
            (r, l) => Some(format!("{r} {} set no request, {l} set no limit", pods(r))),
        }
    }

    /// The note for limits past their capacity. On a node that is over-commitment: normal, said
    /// as such. Against a quota it is not normal, and the note says what it is.
    pub fn limits_note(&self) -> Option<String> {
        if !self.limits_overcommitted() {
            return None;
        }
        let share = self
            .limits_percent()
            .map_or(String::new(), |p| format!(" ({})", whole(p)));
        Some(match self.capacity_kind {
            CapacityKind::Allocatable => {
                format!("limits are over-committed{share}; this is normal")
            }
            CapacityKind::Quota => {
                format!("limits are above the quota{share}: it was lowered or the pods predate it")
            }
        })
    }

    /// The bar of the row, or `None` when nothing caps the resource.
    pub fn bar(&self) -> Option<BarModel> {
        let (basis, amount, scale) = match (&self.capacity, &self.limits, &self.limits_capacity) {
            (Some(capacity), ..) => (BarBasis::Requests, &self.requests, capacity),
            (None, Some(limits), Some(cap)) => (BarBasis::Limits, limits, cap),
            _ => return None,
        };
        let usage = match &self.usage {
            Usage::Value(value) => Some(fraction(value, scale)),
            _ => None,
        };
        // Only a node's limits share the requests' capacity (allocatable). A quota caps limits
        // on their own scale, so a tick on the requests' bar would mislead; the text carries it.
        let limits_mark = match (&self.limits, self.capacity_kind) {
            (Some(limits), CapacityKind::Allocatable) if basis == BarBasis::Requests => {
                Some(fraction(limits, scale))
            }
            _ => None,
        };
        Some(BarModel {
            basis,
            fill: fraction(amount, scale),
            limits_mark,
            usage,
            full: amount >= scale && !amount.is_zero(),
        })
    }

    fn short_text(&self, amount: &Quantity, percent: Option<f64>) -> String {
        let amount = self.kind.format(amount);
        match percent {
            Some(p) => format!("{amount} ({})", whole(p)),
            None => amount,
        }
    }

    fn amount_text(
        &self,
        amount: &Quantity,
        scale: Option<&Quantity>,
        percent: Option<f64>,
    ) -> String {
        let amount = self.kind.format(amount);
        match (scale, percent) {
            (Some(scale), Some(p)) => {
                format!("{amount} of {} ({})", self.kind.format(scale), whole(p))
            }
            (Some(scale), None) => format!("{amount} of {}", self.kind.format(scale)),
            (None, _) => amount,
        }
    }
}

/// `25%`: rounded down, like `kubectl describe node`.
fn whole(percent: f64) -> String {
    format!("{}%", percent.max(0.0).floor() as u64)
}

/// `amount / scale` clamped to `0.0..=1.0`; a zero scale is full when something is asked of it.
fn fraction(amount: &Quantity, scale: &Quantity) -> f32 {
    match percent(amount, scale) {
        Some(p) => (p / 100.0).clamp(0.0, 1.0) as f32,
        None if !amount.is_zero() && scale.is_zero() => 1.0,
        None => 0.0,
    }
}
