//! Tolerations and affinity: where the pod may run.

use std::sync::Arc;

use crate::json::JsonRef;
use crate::view::{arc_of, arr_of, obj_of, str_of, sub};

/// One entry of `spec.tolerations`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toleration {
    /// `key`; empty tolerates every key (with `operator: Exists`).
    pub key: Arc<str>,
    /// `operator`: `Equal` (the default) or `Exists`.
    pub operator: Arc<str>,
    /// `value`.
    pub value: Option<Arc<str>>,
    /// `effect`; empty tolerates every effect.
    pub effect: Option<Arc<str>>,
    /// `tolerationSeconds`: how long a `NoExecute` taint is tolerated.
    pub seconds: Option<i64>,
}

impl Toleration {
    /// `node.kubernetes.io/not-ready:NoExecute op=Exists for 300s`, as `kubectl describe` words it.
    pub fn display(&self) -> String {
        let mut out = String::new();
        if self.key.is_empty() {
            out.push_str("(all taints)");
        } else {
            out.push_str(&self.key);
        }
        if let Some(value) = &self.value {
            out.push('=');
            out.push_str(value);
        }
        if let Some(effect) = &self.effect {
            out.push(':');
            out.push_str(effect);
        }
        out.push_str(&format!(" op={}", self.operator));
        if let Some(seconds) = self.seconds {
            out.push_str(&format!(" for {seconds}s"));
        }
        out
    }
}

pub(super) fn tolerations_of(spec: JsonRef<'_>) -> Vec<Toleration> {
    arr_of(spec, "tolerations")
        .iter()
        .map(|t| Toleration {
            key: Arc::from(str_of(t, "key").unwrap_or("")),
            operator: Arc::from(str_of(t, "operator").unwrap_or("Equal")),
            value: arc_of(t, "value"),
            effect: arc_of(t, "effect"),
            seconds: t.get("tolerationSeconds").and_then(JsonRef::as_i64),
        })
        .collect()
}

/// One line of where-to-schedule rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffinityLine {
    /// `Node selector`, `Node affinity`, `Pod affinity`, `Pod anti-affinity` or `Topology spread`.
    pub kind: &'static str,
    /// `required` or `preferred` (`preferred (weight 50)`); empty for a selector.
    pub strength: String,
    /// The rule: `disktype In [ssd]`, `app=web on kubernetes.io/hostname`.
    pub text: String,
}

pub(super) fn affinity_of(spec: JsonRef<'_>) -> Vec<AffinityLine> {
    let mut lines = Vec::new();
    if let Some(selector) = spec.get("nodeSelector").and_then(JsonRef::as_object) {
        for (key, value) in selector {
            lines.push(AffinityLine {
                kind: "Node selector",
                strength: String::new(),
                text: format!("{key}={}", value.as_str().unwrap_or_default()),
            });
        }
    }
    let affinity = sub(spec, "affinity");
    node_affinity(sub(affinity, "nodeAffinity"), &mut lines);
    for (key, kind) in [
        ("podAffinity", "Pod affinity"),
        ("podAntiAffinity", "Pod anti-affinity"),
    ] {
        pod_affinity(sub(affinity, key), kind, &mut lines);
    }
    for c in arr_of(spec, "topologySpreadConstraints") {
        lines.push(AffinityLine {
            kind: "Topology spread",
            strength: str_of(c, "whenUnsatisfiable")
                .unwrap_or("DoNotSchedule")
                .to_owned(),
            text: format!(
                "maxSkew {} over {}",
                c.get("maxSkew").and_then(JsonRef::as_i64).unwrap_or(1),
                str_of(c, "topologyKey").unwrap_or("?")
            ),
        });
    }
    lines
}

fn node_affinity(node: JsonRef<'_>, lines: &mut Vec<AffinityLine>) {
    let required = sub(node, "requiredDuringSchedulingIgnoredDuringExecution");
    for term in arr_of(required, "nodeSelectorTerms") {
        lines.push(AffinityLine {
            kind: "Node affinity",
            strength: "required".to_owned(),
            text: expressions(term),
        });
    }
    for pref in arr_of(node, "preferredDuringSchedulingIgnoredDuringExecution") {
        lines.push(AffinityLine {
            kind: "Node affinity",
            strength: weight(pref),
            text: expressions(sub(pref, "preference")),
        });
    }
}

fn pod_affinity(pod: JsonRef<'_>, kind: &'static str, lines: &mut Vec<AffinityLine>) {
    for term in arr_of(pod, "requiredDuringSchedulingIgnoredDuringExecution") {
        lines.push(AffinityLine {
            kind,
            strength: "required".to_owned(),
            text: pod_term(term),
        });
    }
    for pref in arr_of(pod, "preferredDuringSchedulingIgnoredDuringExecution") {
        lines.push(AffinityLine {
            kind,
            strength: weight(pref),
            text: pod_term(sub(pref, "podAffinityTerm")),
        });
    }
}

fn weight(preference: JsonRef<'_>) -> String {
    let weight = preference
        .get("weight")
        .and_then(JsonRef::as_i64)
        .unwrap_or(0);
    format!("preferred (weight {weight})")
}

/// `app=web, tier In [a,b] on kubernetes.io/hostname`.
fn pod_term(term: JsonRef<'_>) -> String {
    let selector = match obj_of(term, "labelSelector") {
        Some(selector) => {
            let mut parts = Vec::new();
            if let Some(labels) = selector.get("matchLabels").and_then(JsonRef::as_object) {
                parts.extend(
                    labels
                        .iter()
                        .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or_default())),
                );
            }
            let rest = expressions_in(selector, "matchExpressions");
            if !rest.is_empty() {
                parts.push(rest);
            }
            parts.join(", ")
        }
        None => "any pod".to_owned(),
    };
    format!(
        "{selector} on {}",
        str_of(term, "topologyKey").unwrap_or("?")
    )
}

/// A node selector term's `matchExpressions` and `matchFields`.
fn expressions(term: JsonRef<'_>) -> String {
    let parts = [
        expressions_in(term, "matchExpressions"),
        expressions_in(term, "matchFields"),
    ];
    parts
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

fn expressions_in(parent: JsonRef<'_>, key: &str) -> String {
    arr_of(parent, key)
        .iter()
        .map(|e| {
            let values: Vec<&str> = arr_of(e, "values")
                .iter()
                .filter_map(JsonRef::as_str)
                .collect();
            let op = str_of(e, "operator").unwrap_or("?");
            let key = str_of(e, "key").unwrap_or("?");
            if values.is_empty() {
                format!("{key} {op}")
            } else {
                format!("{key} {op} [{}]", values.join(","))
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}
