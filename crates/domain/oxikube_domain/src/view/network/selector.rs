//! [`selector_text`]: a label selector as text.

use super::super::{arr_of, str_of, sub};
use crate::json::JsonRef;

/// A `LabelSelector` (`matchLabels`, `matchExpressions`) as text: `app=web, tier in (a,b)`.
/// Labels come first, sorted by key, then the expressions in their order. `None` when the
/// selector is absent or empty (which, for a pod selector, selects everything).
pub fn selector_text(selector: JsonRef<'_>) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(labels) = sub(selector, "matchLabels").as_object() {
        let mut pairs: Vec<(&str, &str)> = labels
            .iter()
            .filter_map(|(key, value)| Some((key, value.as_str()?)))
            .collect();
        pairs.sort_unstable();
        parts.extend(pairs.into_iter().map(|(k, v)| format!("{k}={v}")));
    }
    for expression in arr_of(selector, "matchExpressions") {
        let Some(key) = str_of(expression, "key") else {
            continue;
        };
        let values = || {
            arr_of(expression, "values")
                .iter()
                .filter_map(JsonRef::as_str)
                .collect::<Vec<_>>()
                .join(",")
        };
        parts.push(match str_of(expression, "operator") {
            Some("In") => format!("{key} in ({})", values()),
            Some("NotIn") => format!("{key} notin ({})", values()),
            Some("DoesNotExist") => format!("!{key}"),
            _ => key.to_owned(),
        });
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}
