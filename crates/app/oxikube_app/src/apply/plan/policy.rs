//! What the guard says about a `resource::ApplyPlan`: the confirmation text and the audit detail.
//! Counts and kind names only, never a field of an object (they may be Secrets).

use std::collections::BTreeMap;

use oxikube_domain::apply::ManifestObjects;
use oxikube_domain::command::Command;
use serde_json::Value;

use crate::apply::handler::DEFAULT_FIELD_MANAGER;

/// How many kinds the confirmation and the audit detail list before they fold the rest.
const MAX_KINDS: usize = 8;

/// The one-line confirmation of a plan, `None` for any other command: "Apply 12 objects on
/// kind-dev: 2 Namespace, 1 CustomResourceDefinition, 9 ConfigMap. ...".
pub(crate) fn summary(command: &Command, context: &str) -> Option<String> {
    let Command::ResourceApplyPlan {
        objects,
        force_conflicts,
        stop_on_error,
        ..
    } = command
    else {
        return None;
    };
    let count = match objects.len() {
        1 => "1 object".to_owned(),
        n => format!("{n} objects"),
    };
    let kinds: Vec<String> = kinds(objects)
        .into_iter()
        .map(|(kind, n)| format!("{n} {kind}"))
        .collect();
    let mut text = format!(
        "Apply {count} on {context}: {}. Each object is server-side applied in kind order after \
         a dry run, and audited on its own.",
        kinds.join(", ")
    );
    if *force_conflicts {
        text.push_str(" Fields other managers own are taken over (force).");
    }
    if *stop_on_error {
        text.push_str(" The first failure stops the rest.");
    }
    Some(text)
}

/// The audit detail of a plan's own record, `None` for any other command:
/// `items=12 kinds=ConfigMap:9,Namespace:2 manager=oxikube force=false stop_on_error=false`. The
/// handler adds the batch id and the counts of what happened.
pub(crate) fn detail(command: &Command) -> Option<String> {
    let Command::ResourceApplyPlan {
        objects,
        field_manager,
        force_conflicts,
        stop_on_error,
        ..
    } = command
    else {
        return None;
    };
    let kinds: Vec<String> = kinds(objects)
        .into_iter()
        .map(|(kind, n)| format!("{kind}:{n}"))
        .collect();
    let manager = field_manager.as_deref().unwrap_or(DEFAULT_FIELD_MANAGER);
    Some(format!(
        "items={} kinds={} manager={manager} force={force_conflicts} \
         stop_on_error={stop_on_error}",
        objects.len(),
        kinds.join(",")
    ))
}

/// Kinds and how many objects of each, most first (then by name), at most [`MAX_KINDS`]. A kind
/// that is not a plain name is left out: it is the caller's text.
fn kinds(objects: &ManifestObjects) -> Vec<(&str, usize)> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for kind in objects
        .iter()
        .filter_map(|object| object.get("kind").and_then(Value::as_str))
        .filter(|kind| kind.len() <= 40 && kind.chars().all(|c| c.is_ascii_alphanumeric()))
    {
        *counts.entry(kind).or_default() += 1;
    }
    let mut kinds: Vec<(&str, usize)> = counts.into_iter().collect();
    kinds.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    kinds.truncate(MAX_KINDS);
    kinds
}
