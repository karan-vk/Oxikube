//! [`WorkloadDetail`]: what a Deployment's, StatefulSet's, ReplicaSet's or ReplicationController's
//! detail panel shows beyond its replica counts: the pod template, the update strategy, the
//! selector and the conditions with their reasons (E12-S04).
//!
//! Like every view-model here it reads the JSON by key and degrades on gaps; it is built once per
//! object version by the panel, never in a render. The cheap [`workload_conditions`] and
//! [`conditions_summary`] serve the table's Conditions column without building the rest.

use std::sync::Arc;

use super::{
    Condition, ConditionStatus, ViewError, WorkloadKind, arc_of, arr_of, check_kind, obj_of,
    opt_count, str_of, sub,
};
use crate::json::JsonRef;
use crate::resource::Resource;

/// How much a condition should worry the reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConditionLevel {
    /// Nothing to say (no conditions).
    Neutral,
    /// The condition is as it should be.
    Ok,
    /// Short of what is wanted, or not known: `Available` is `False`, a status is `Unknown`.
    Warn,
    /// A fault: `Progressing` is `False`, `ReplicaFailure` is `True`.
    Error,
}

/// One entry of a controller's `status.conditions`, with the level it deserves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadCondition {
    /// The condition as the API reports it.
    pub condition: Condition,
    /// How it reads: a Deployment's `Available=True` is [`Ok`](ConditionLevel::Ok), its
    /// `ReplicaFailure=True` is an [`Error`](ConditionLevel::Error).
    pub level: ConditionLevel,
}

impl WorkloadCondition {
    fn new(condition: Condition) -> Self {
        let level = level_of(&condition);
        Self { condition, level }
    }

    /// `Available` for a condition that holds, `Available=False` (or `=Unknown`) otherwise: the
    /// badge text of the table's Conditions column.
    pub fn badge(&self) -> String {
        match self.condition.status {
            ConditionStatus::True => self.condition.kind.to_string(),
            other => format!("{}={}", self.condition.kind, other.as_str()),
        }
    }
}

/// Condition types whose `True` is the bad case.
const NEGATIVE: &[&str] = &["ReplicaFailure"];

fn level_of(condition: &Condition) -> ConditionLevel {
    let negative = NEGATIVE.contains(&&*condition.kind);
    match (condition.status, negative) {
        (ConditionStatus::True, false) | (ConditionStatus::False, true) => ConditionLevel::Ok,
        (ConditionStatus::True, true) => ConditionLevel::Error,
        (ConditionStatus::False, false) if &*condition.kind == "Available" => ConditionLevel::Warn,
        (ConditionStatus::False, false) => ConditionLevel::Error,
        (ConditionStatus::Unknown, _) => ConditionLevel::Warn,
    }
}

/// The conditions of `res` in API order. Empty for a kind without any (a StatefulSet usually
/// reports none) and for a metadata-only object.
pub fn workload_conditions(res: &Resource) -> Vec<WorkloadCondition> {
    arr_of(sub(res.json(), "status"), "conditions")
        .iter()
        .filter_map(Condition::from_json)
        .map(WorkloadCondition::new)
        .collect()
}

/// The text and the worst level of `conditions`: the badges joined with commas
/// (`Available,Progressing`), [`ConditionLevel::Neutral`] and no text when there are none.
pub fn conditions_summary(conditions: &[WorkloadCondition]) -> (String, ConditionLevel) {
    let mut text = String::new();
    let mut worst = ConditionLevel::Neutral;
    for condition in conditions {
        if !text.is_empty() {
            text.push(',');
        }
        text.push_str(&condition.badge());
        worst = worst.max(condition.level);
    }
    (text, worst)
}

/// A pod selector as the controller declares it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LabelSelector {
    /// `matchLabels` in key order (a ReplicationController's whole `spec.selector`).
    pub match_labels: Vec<(Arc<str>, Arc<str>)>,
    /// `matchExpressions` as text: `tier in (web,api)`, `canary`, `!canary`, `env notin (dev)`.
    pub match_expressions: Vec<String>,
}

impl LabelSelector {
    /// Whether it selects everything (no labels, no expressions).
    pub fn is_empty(&self) -> bool {
        self.match_labels.is_empty() && self.match_expressions.is_empty()
    }

    /// The selector as `kubectl` prints it: `app=web,tier in (a,b)`.
    pub fn display(&self) -> String {
        let labels = self.match_labels.iter().map(|(k, v)| format!("{k}={v}"));
        labels
            .chain(self.match_expressions.iter().cloned())
            .collect::<Vec<_>>()
            .join(",")
    }

    fn from_json(selector: JsonRef<'_>, plain_map: bool) -> Self {
        let labels = if plain_map {
            Some(selector)
        } else {
            selector.get("matchLabels")
        };
        let match_labels = labels
            .and_then(JsonRef::as_object)
            .map(|map| {
                map.iter()
                    .filter_map(|(k, v)| Some((Arc::from(k), Arc::from(v.as_str()?))))
                    .collect()
            })
            .unwrap_or_default();
        let match_expressions = if plain_map {
            Vec::new()
        } else {
            arr_of(selector, "matchExpressions")
                .iter()
                .filter_map(expression)
                .collect()
        };
        Self {
            match_labels,
            match_expressions,
        }
    }
}

fn expression(requirement: JsonRef<'_>) -> Option<String> {
    let key = str_of(requirement, "key")?;
    let values = || {
        arr_of(requirement, "values")
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(",")
    };
    Some(match str_of(requirement, "operator")? {
        "In" => format!("{key} in ({})", values()),
        "NotIn" => format!("{key} notin ({})", values()),
        "Exists" => key.to_owned(),
        "DoesNotExist" => format!("!{key}"),
        other => format!("{key} {other} ({})", values()),
    })
}

/// The update strategy: its type and the parameters that apply to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Strategy {
    /// `RollingUpdate`, `Recreate` or `OnDelete`.
    pub kind: Arc<str>,
    /// Labelled parameters in display order: `("Max surge", "25%")`, `("Partition", "3")`.
    pub parameters: Vec<(&'static str, String)>,
}

/// One container of the pod template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateContainer {
    /// Container name.
    pub name: Arc<str>,
    /// Image reference.
    pub image: Arc<str>,
    /// Whether it is an init container.
    pub init: bool,
    /// Declared ports as `http:8080/TCP` (name omitted when unnamed).
    pub ports: Vec<String>,
    /// `resources.requests` in key order.
    pub requests: Vec<(Arc<str>, Arc<str>)>,
    /// `resources.limits` in key order.
    pub limits: Vec<(Arc<str>, Arc<str>)>,
}

/// The detail panel's view of a replica-based controller. See the [module docs](self).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadDetail {
    /// Which controller.
    pub kind: WorkloadKind,
    /// The pod selector.
    pub selector: LabelSelector,
    /// The update strategy; `None` for a ReplicaSet or ReplicationController, which have none.
    pub strategy: Option<Strategy>,
    /// Init containers first, then the containers, of `spec.template.spec`.
    pub containers: Vec<TemplateContainer>,
    /// `status.conditions`.
    pub conditions: Vec<WorkloadCondition>,
    /// `spec.minReadySeconds`.
    pub min_ready_seconds: Option<u32>,
    /// `spec.revisionHistoryLimit`.
    pub revision_history_limit: Option<u32>,
    /// A StatefulSet's governing Service (`spec.serviceName`).
    pub service_name: Option<Arc<str>>,
}

impl WorkloadDetail {
    /// The detail of a Deployment, StatefulSet, ReplicaSet or ReplicationController.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] for any other kind (a DaemonSet included: it is not scalable and
    /// has its own panel).
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(
            res,
            "Deployment, StatefulSet, ReplicaSet or ReplicationController",
            &[
                ("apps", "Deployment"),
                ("apps", "StatefulSet"),
                ("apps", "ReplicaSet"),
                ("", "ReplicationController"),
            ],
        )?;
        let kind = WorkloadKind::of(&res.kind).ok_or_else(|| ViewError::WrongKind {
            expected: "a replica controller",
            found: res.kind.kind.to_string(),
        })?;
        let spec = sub(res.json(), "spec");
        let pod = sub(sub(spec, "template"), "spec");
        let plain = kind == WorkloadKind::ReplicationController;
        let selector = LabelSelector::from_json(sub(spec, "selector"), plain);
        let containers = arr_of(pod, "initContainers")
            .iter()
            .map(|c| container(c, true))
            .chain(
                arr_of(pod, "containers")
                    .iter()
                    .map(|c| container(c, false)),
            )
            .collect();
        Ok(Self {
            kind,
            selector,
            strategy: strategy(kind, spec),
            containers,
            conditions: workload_conditions(res),
            min_ready_seconds: opt_count(spec, "minReadySeconds"),
            revision_history_limit: opt_count(spec, "revisionHistoryLimit"),
            service_name: (kind == WorkloadKind::StatefulSet)
                .then(|| arc_of(spec, "serviceName"))
                .flatten(),
        })
    }
}

fn container(c: JsonRef<'_>, init: bool) -> TemplateContainer {
    let ports = arr_of(c, "ports")
        .iter()
        .filter_map(|port| {
            let number = port.get("containerPort")?.as_u64()?;
            let protocol = str_of(port, "protocol").unwrap_or("TCP");
            Some(match str_of(port, "name") {
                Some(name) => format!("{name}:{number}/{protocol}"),
                None => format!("{number}/{protocol}"),
            })
        })
        .collect();
    let resources = sub(c, "resources");
    TemplateContainer {
        name: arc_of(c, "name").unwrap_or_else(|| Arc::from("")),
        image: arc_of(c, "image").unwrap_or_else(|| Arc::from("")),
        init,
        ports,
        requests: quantities(resources, "requests"),
        limits: quantities(resources, "limits"),
    }
}

fn quantities(resources: JsonRef<'_>, key: &str) -> Vec<(Arc<str>, Arc<str>)> {
    obj_of(resources, key)
        .and_then(JsonRef::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| Some((Arc::from(k), Arc::from(v.as_str()?))))
                .collect()
        })
        .unwrap_or_default()
}

/// A number or a percentage string (`maxSurge: 1`, `maxUnavailable: 25%`) as text.
fn int_or_percent(v: JsonRef<'_>, key: &str) -> Option<String> {
    let v = v.get(key)?;
    v.as_str()
        .map(str::to_owned)
        .or_else(|| v.as_u64().map(|n| n.to_string()))
}

fn strategy(kind: WorkloadKind, spec: JsonRef<'_>) -> Option<Strategy> {
    match kind {
        WorkloadKind::Deployment => {
            let strategy = sub(spec, "strategy");
            let kind: Arc<str> =
                arc_of(strategy, "type").unwrap_or_else(|| Arc::from("RollingUpdate"));
            let mut parameters = Vec::new();
            if &*kind == "RollingUpdate" {
                let rolling = sub(strategy, "rollingUpdate");
                parameters.push((
                    "Max surge",
                    int_or_percent(rolling, "maxSurge").unwrap_or_else(|| "25%".into()),
                ));
                parameters.push((
                    "Max unavailable",
                    int_or_percent(rolling, "maxUnavailable").unwrap_or_else(|| "25%".into()),
                ));
            }
            if let Some(seconds) = opt_count(spec, "progressDeadlineSeconds") {
                parameters.push(("Progress deadline", format!("{seconds}s")));
            }
            Some(Strategy { kind, parameters })
        }
        WorkloadKind::StatefulSet => {
            let strategy = sub(spec, "updateStrategy");
            let kind: Arc<str> =
                arc_of(strategy, "type").unwrap_or_else(|| Arc::from("RollingUpdate"));
            let mut parameters = Vec::new();
            if &*kind == "RollingUpdate" {
                let rolling = sub(strategy, "rollingUpdate");
                parameters.push((
                    "Partition",
                    opt_count(rolling, "partition").unwrap_or(0).to_string(),
                ));
                if let Some(max) = int_or_percent(rolling, "maxUnavailable") {
                    parameters.push(("Max unavailable", max));
                }
            }
            if let Some(policy) = str_of(spec, "podManagementPolicy") {
                parameters.push(("Pod management", policy.to_owned()));
            }
            Some(Strategy { kind, parameters })
        }
        WorkloadKind::ReplicaSet
        | WorkloadKind::ReplicationController
        | WorkloadKind::DaemonSet => None,
    }
}
