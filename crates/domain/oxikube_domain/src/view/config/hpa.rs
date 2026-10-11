//! [`HpaSummary`]: a HorizontalPodAutoscaler's scale target, bounds, metrics and state.
//!
//! `autoscaling/v2` is the model: `spec.metrics` with `status.currentMetrics`. An `autoscaling/v1`
//! object (only a CPU percentage) reads as one `cpu` resource metric, so the table and the panel
//! need no second code path.

use std::sync::Arc;

use jiff::Timestamp;

use super::super::{
    Condition, ConditionStatus, ViewError, arr_of, check_kind, i64_of, str_of, sub, ts_of,
};
use super::util::TargetRef;
use crate::json::JsonRef;
use crate::resource::Resource;

/// The kind of an HPA metric source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HpaMetricKind {
    /// A pod resource (`cpu`, `memory`) averaged over the pods.
    Resource,
    /// A resource of one container.
    ContainerResource,
    /// A custom per-pod metric.
    Pods,
    /// A metric of another object.
    Object,
    /// A metric from outside the cluster.
    External,
}

impl HpaMetricKind {
    /// The API `type`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Resource => "Resource",
            Self::ContainerResource => "ContainerResource",
            Self::Pods => "Pods",
            Self::Object => "Object",
            Self::External => "External",
        }
    }

    fn body_key(self) -> &'static str {
        match self {
            Self::Resource => "resource",
            Self::ContainerResource => "containerResource",
            Self::Pods => "pods",
            Self::Object => "object",
            Self::External => "external",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "Resource" => Self::Resource,
            "ContainerResource" => Self::ContainerResource,
            "Pods" => Self::Pods,
            "Object" => Self::Object,
            "External" => Self::External,
            _ => return None,
        })
    }
}

/// One metric of an HPA: what it measures, its target and its current value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HpaMetric {
    /// The source kind.
    pub kind: HpaMetricKind,
    /// The measured thing: `cpu`, `memory`, or a custom metric's name.
    pub name: Arc<str>,
    /// What it is measured on besides the pods: a container name, or `Kind/name` of an object.
    pub on: Option<Arc<str>>,
    /// The metric's label selector as text, when it has one (two metrics may share a name).
    pub selector: Option<String>,
    /// The target: `80%`, `512Mi`, `100`.
    pub target: Option<String>,
    /// The latest value: `42%`, `300Mi`. `None` until the controller has read it.
    pub current: Option<String>,
}

impl HpaMetric {
    /// `current/target` as kubectl prints a metric (`42%/80%`, `<unknown>/512Mi`).
    pub fn text(&self) -> String {
        format!("{}/{}", self.current_text(), self.target_text())
    }

    /// The current value, `<unknown>` until the controller reports one.
    pub fn current_text(&self) -> &str {
        self.current.as_deref().unwrap_or("<unknown>")
    }

    /// The target value, `<unknown>` when the spec sets none.
    pub fn target_text(&self) -> &str {
        self.target.as_deref().unwrap_or("<unknown>")
    }

    /// `kind name`, with what it is measured on and its selector: `Resource cpu`,
    /// `Object requests-per-second on Ingress/web`, `External queue_depth [env=prod]`.
    pub fn title(&self) -> String {
        let mut out = format!("{} {}", self.kind.as_str(), self.name);
        if let Some(on) = &self.on {
            out.push_str(&format!(" on {on}"));
        }
        if let Some(selector) = &self.selector {
            out.push_str(&format!(" [{selector}]"));
        }
        out
    }
}

/// How the autoscaler is doing, from its conditions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HpaState {
    /// `ScalingActive` and not limited.
    Active,
    /// Scaling, but held at a bound (`ScalingLimited`): the reason says which.
    Limited(Arc<str>),
    /// It cannot scale: no metrics, or `AbleToScale` false. The reason says why.
    Inactive(Arc<str>),
    /// Scaling is switched off because the target is scaled to zero (`ScalingDisabled`): not a
    /// fault.
    Disabled,
    /// No conditions yet.
    Unknown,
}

impl HpaState {
    /// The text of a `Status` cell.
    pub fn text(&self) -> &str {
        match self {
            Self::Active => "Active",
            Self::Limited(reason) | Self::Inactive(reason) => reason,
            Self::Disabled => "ScalingDisabled",
            Self::Unknown => "Unknown",
        }
    }
}

/// A HorizontalPodAutoscaler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HpaSummary {
    /// `spec.scaleTargetRef`.
    pub reference: Option<TargetRef>,
    /// `spec.minReplicas`; the API default is 1.
    pub min_replicas: i64,
    /// `spec.maxReplicas`.
    pub max_replicas: Option<i64>,
    /// `status.currentReplicas`.
    pub current_replicas: Option<i64>,
    /// `status.desiredReplicas`.
    pub desired_replicas: Option<i64>,
    /// The metrics, in spec order, each with its latest value.
    pub metrics: Vec<HpaMetric>,
    /// `status.conditions`.
    pub conditions: Vec<Condition>,
    /// `status.lastScaleTime`.
    pub last_scale: Option<Timestamp>,
    /// How it is doing.
    pub state: HpaState,
}

impl HpaSummary {
    /// The summary of a `HorizontalPodAutoscaler` (`autoscaling/v2` or `v1`).
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not an HPA.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(
            res,
            "HorizontalPodAutoscaler",
            &[("autoscaling", "HorizontalPodAutoscaler")],
        )?;
        let (spec, status) = (sub(res.json(), "spec"), sub(res.json(), "status"));
        let current = arr_of(status, "currentMetrics");
        let mut metrics: Vec<HpaMetric> = arr_of(spec, "metrics")
            .iter()
            .filter_map(|m| read_metric(m, current.iter()))
            .collect();
        if metrics.is_empty()
            && let Some(target) = i64_of(spec, "targetCPUUtilizationPercentage")
        {
            // `autoscaling/v1`: one CPU utilisation target.
            metrics.push(HpaMetric {
                kind: HpaMetricKind::Resource,
                name: Arc::from("cpu"),
                on: None,
                selector: None,
                target: Some(format!("{target}%")),
                current: i64_of(status, "currentCPUUtilizationPercentage").map(|c| format!("{c}%")),
            });
        }
        let conditions: Vec<Condition> = arr_of(status, "conditions")
            .iter()
            .filter_map(Condition::from_json)
            .collect();
        let state = state_of(&conditions);
        Ok(Self {
            reference: spec.get("scaleTargetRef").and_then(TargetRef::read),
            min_replicas: i64_of(spec, "minReplicas").unwrap_or(1),
            max_replicas: i64_of(spec, "maxReplicas"),
            current_replicas: i64_of(status, "currentReplicas"),
            desired_replicas: i64_of(status, "desiredReplicas"),
            metrics,
            conditions,
            last_scale: ts_of(status, "lastScaleTime"),
            state,
        })
    }

    /// The `Metrics` cell: every metric as `current/target`, comma-separated.
    pub fn metrics_text(&self) -> Option<String> {
        if self.metrics.is_empty() {
            return None;
        }
        Some(
            self.metrics
                .iter()
                .map(HpaMetric::text)
                .collect::<Vec<_>>()
                .join(", "),
        )
    }
}

fn state_of(conditions: &[Condition]) -> HpaState {
    let find = |kind: &str| conditions.iter().find(|c| &*c.kind == kind);
    let reason =
        |c: &Condition, fallback: &str| c.reason.clone().unwrap_or_else(|| Arc::from(fallback));
    if let Some(c) = find("AbleToScale").filter(|c| c.status == ConditionStatus::False) {
        return HpaState::Inactive(reason(c, "NotAbleToScale"));
    }
    if let Some(c) = find("ScalingActive").filter(|c| c.status == ConditionStatus::False) {
        // A target scaled to zero switches the autoscaler off on purpose.
        if c.reason.as_deref() == Some("ScalingDisabled") {
            return HpaState::Disabled;
        }
        return HpaState::Inactive(reason(c, "NotScaling"));
    }
    if let Some(c) = find("ScalingLimited").filter(|c| c.is_true()) {
        return HpaState::Limited(reason(c, "ScalingLimited"));
    }
    if find("ScalingActive").is_some_and(Condition::is_true) {
        return HpaState::Active;
    }
    HpaState::Unknown
}

/// The metric `m` of the spec, matched with its latest value in `current`.
fn read_metric<'a>(
    m: JsonRef<'a>,
    mut current: impl Iterator<Item = JsonRef<'a>>,
) -> Option<HpaMetric> {
    let kind = HpaMetricKind::parse(str_of(m, "type")?)?;
    let body = m.get(kind.body_key())?;
    let id = identity(kind, body);
    let latest = current.find(|c| {
        str_of(*c, "type").and_then(HpaMetricKind::parse) == Some(kind)
            && c.get(kind.body_key())
                .is_some_and(|b| identity(kind, b) == id)
    });
    let (name, on, selector) = id;
    Some(HpaMetric {
        kind,
        target: value_text(body.get("target")),
        current: latest.and_then(|c| value_text(c.get(kind.body_key())?.get("current"))),
        name,
        on,
        selector,
    })
}

/// What tells two metrics apart: the name, what it is measured on, and the label selector.
type MetricId = (Arc<str>, Option<Arc<str>>, Option<String>);

fn identity(kind: HpaMetricKind, body: JsonRef<'_>) -> MetricId {
    let metric = body.get("metric");
    let name = str_of(body, "name")
        .or_else(|| metric.and_then(|m| str_of(m, "name")))
        .unwrap_or_default();
    let on = match kind {
        HpaMetricKind::ContainerResource => str_of(body, "container").map(Arc::from),
        HpaMetricKind::Object => body.get("describedObject").and_then(|o| {
            Some(Arc::from(format!(
                "{}/{}",
                str_of(o, "kind")?,
                str_of(o, "name")?
            )))
        }),
        _ => None,
    };
    let selector = metric
        .and_then(|m| m.get("selector"))
        .and_then(super::util::selector_text)
        .filter(|s| s != "<all>");
    (Arc::from(name), on, selector)
}

/// A `MetricTarget` or `MetricValueStatus`: utilisation as a percentage, else the average value,
/// else the value.
fn value_text(v: Option<JsonRef<'_>>) -> Option<String> {
    let v = v?;
    if let Some(u) = i64_of(v, "averageUtilization") {
        return Some(format!("{u}%"));
    }
    str_of(v, "averageValue")
        .or_else(|| str_of(v, "value"))
        .map(str::to_owned)
}
