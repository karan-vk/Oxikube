//! [`AggregationRule`]: the label selectors an aggregated ClusterRole collects rules with.

use std::sync::Arc;

use crate::json::JsonRef;
use crate::resource::StrMap;

/// A ClusterRole's `aggregationRule`: the ClusterRoles whose labels match any selector
/// contribute their rules.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AggregationRule {
    /// `clusterRoleSelectors`; an empty list selects nothing.
    pub selectors: Vec<RoleSelector>,
}

impl AggregationRule {
    /// Reads `aggregationRule` from its JSON.
    pub fn from_json(rule: JsonRef<'_>) -> Self {
        let selectors = rule
            .get("clusterRoleSelectors")
            .and_then(JsonRef::as_array)
            .map(|items| items.iter().map(RoleSelector::from_json).collect())
            .unwrap_or_default();
        Self { selectors }
    }

    /// Whether a ClusterRole with `labels` contributes (any selector matches).
    pub fn selects(&self, labels: &StrMap) -> bool {
        self.selectors
            .iter()
            .any(|selector| selector.matches(labels))
    }
}

/// One `metav1.LabelSelector`: every `matchLabels` pair and every expression must hold. An
/// empty selector matches every ClusterRole, as the API server's does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoleSelector {
    /// `matchLabels`.
    pub match_labels: Vec<(Arc<str>, Arc<str>)>,
    /// `matchExpressions`.
    pub match_expressions: Vec<SelectorRequirement>,
}

/// One `matchExpressions` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorRequirement {
    /// The label key.
    pub key: Arc<str>,
    /// `In`, `NotIn`, `Exists` or `DoesNotExist`; anything else never matches.
    pub operator: Arc<str>,
    /// The values `In` and `NotIn` compare against.
    pub values: Vec<Arc<str>>,
}

impl RoleSelector {
    /// Reads one selector from its JSON.
    pub fn from_json(selector: JsonRef<'_>) -> Self {
        let match_labels = selector
            .get("matchLabels")
            .and_then(JsonRef::as_object)
            .map(|labels| {
                labels
                    .iter()
                    .filter_map(|(key, value)| Some((Arc::from(key), Arc::from(value.as_str()?))))
                    .collect()
            })
            .unwrap_or_default();
        let match_expressions = selector
            .get("matchExpressions")
            .and_then(JsonRef::as_array)
            .map(|items| items.iter().filter_map(requirement).collect())
            .unwrap_or_default();
        Self {
            match_labels,
            match_expressions,
        }
    }

    /// Whether `labels` satisfy the selector.
    pub fn matches(&self, labels: &StrMap) -> bool {
        let label = |key: &str| labels.get(key).map(|value| &**value);
        self.match_labels
            .iter()
            .all(|(key, value)| label(key) == Some(&**value))
            && self.match_expressions.iter().all(|expression| {
                let found = label(&expression.key);
                let listed = || found.is_some_and(|v| expression.values.iter().any(|x| **x == *v));
                match &*expression.operator {
                    "In" => listed(),
                    "NotIn" => !listed(),
                    "Exists" => found.is_some(),
                    "DoesNotExist" => found.is_none(),
                    _ => false,
                }
            })
    }
}

fn requirement(expression: JsonRef<'_>) -> Option<SelectorRequirement> {
    let text = |key: &str| expression.get(key).and_then(JsonRef::as_str);
    Some(SelectorRequirement {
        key: text("key")?.into(),
        operator: text("operator")?.into(),
        values: expression
            .get("values")
            .and_then(JsonRef::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(JsonRef::as_str)
                    .map(Arc::from)
                    .collect()
            })
            .unwrap_or_default(),
    })
}
