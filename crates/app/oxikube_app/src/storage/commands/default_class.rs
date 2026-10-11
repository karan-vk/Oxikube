//! The `storageclass::SetDefault` handler.

use std::sync::Arc;

use oxikube_domain::command::Command;
use oxikube_domain::view::storage::{
    BETA_DEFAULT_CLASS_ANNOTATION, DEFAULT_CLASS_ANNOTATION, is_default_class,
};
use oxikube_domain::{OxiError, OxiResult, Resource};
use oxikube_ports::{ListOptions, Patch, PatchKind, ResourceReader, WriteOptions};
use serde_json::{Value, json};

use crate::command_bus::{CommandOutput, HandlerContext};
use crate::session::ClusterSessionManager;
use crate::storage::read::class_gvk;

pub(super) async fn handle(
    command: Command,
    cx: HandlerContext,
    sessions: ClusterSessionManager,
) -> OxiResult<CommandOutput> {
    let Command::StorageClassSetDefault {
        target,
        clear_previous,
    } = command
    else {
        return Err(OxiError::validation(
            "not a storageclass::SetDefault command",
        ));
    };
    if &*target.gvk.group != "storage.k8s.io" || &*target.gvk.kind != "StorageClass" {
        return Err(OxiError::validation(format!(
            "storageclass::SetDefault sets a StorageClass, not a {}",
            target.gvk.kind
        )));
    }
    let mutation = cx.require_mutation()?;
    let reader: Arc<dyn ResourceReader> = sessions
        .get(&target.cluster)
        .and_then(|session| session.resources())
        .ok_or_else(|| OxiError::not_found("the cluster is not connected"))?;

    let classes = reader
        .list(&class_gvk(), None, &ListOptions::default())
        .await?
        .items;
    let chosen = classes
        .iter()
        .find(|class| *class.meta.name == *target.name)
        .ok_or_else(|| OxiError::not_found(format!("StorageClass {} not found", target.name)))?;
    let others: Vec<&Resource> = classes
        .iter()
        .filter(|class| class.meta.name != chosen.meta.name && is_default_class(&class.meta))
        .collect();
    let chosen_is_default = is_default_class(&chosen.meta);
    let clearing: Vec<&Resource> = if clear_previous { others } else { Vec::new() };
    if chosen_is_default && clearing.is_empty() {
        return Ok(CommandOutput {
            message: Some(format!(
                "{} is the default StorageClass already",
                target.name
            )),
            data: Some(json!({"outcome": "unchanged", "set": &*target.name, "cleared": []})),
        });
    }

    // The new default first: if clearing the old one then fails, the cluster has two defaults
    // (it still provisions) rather than none.
    let mut writes: Vec<(&str, Patch)> = Vec::new();
    if !chosen_is_default {
        writes.push((&chosen.meta.name, annotate(true, false)));
    }
    for class in &clearing {
        let beta = class
            .meta
            .annotations
            .contains_key(BETA_DEFAULT_CLASS_ANNOTATION);
        writes.push((&class.meta.name, annotate(false, beta)));
    }

    let writer = mutation.writer();
    let options = mutation.write_options();
    let rehearsal = WriteOptions {
        dry_run: true,
        ..options.clone()
    };
    for (name, patch) in &writes {
        writer
            .patch(&class_gvk(), None, name, patch, &rehearsal)
            .await?;
    }
    let cleared: Vec<&str> = clearing.iter().map(|c| &*c.meta.name as &str).collect();
    if mutation.dry_run() {
        return Ok(CommandOutput {
            message: Some(format!(
                "Dry run: {} can become the default StorageClass",
                target.name
            )),
            data: Some(summary("dry_run", &target.name, &cleared)),
        });
    }
    for (name, patch) in &writes {
        writer
            .patch(&class_gvk(), None, name, patch, &options)
            .await?;
    }
    let message = if cleared.is_empty() {
        format!("{} is now the default StorageClass", target.name)
    } else {
        format!(
            "{} is now the default StorageClass (no longer {})",
            target.name,
            cleared.join(", ")
        )
    };
    Ok(CommandOutput {
        message: Some(message),
        data: Some(summary("set", &target.name, &cleared)),
    })
}

/// The merge patch that turns the default annotation on or off. Turning it off writes `"false"`
/// (the documented way) and drops the legacy beta annotation when the class carries one, so it
/// cannot keep the class default.
fn annotate(default: bool, drop_beta: bool) -> Patch {
    let mut annotations = serde_json::Map::new();
    annotations.insert(
        DEFAULT_CLASS_ANNOTATION.to_owned(),
        json!(if default { "true" } else { "false" }),
    );
    if drop_beta {
        annotations.insert(BETA_DEFAULT_CLASS_ANNOTATION.to_owned(), Value::Null);
    }
    Patch {
        kind: PatchKind::Merge,
        body: json!({"metadata": {"annotations": annotations}}),
    }
}

fn summary(outcome: &str, set: &str, cleared: &[&str]) -> Value {
    json!({"outcome": outcome, "set": set, "cleared": cleared})
}
