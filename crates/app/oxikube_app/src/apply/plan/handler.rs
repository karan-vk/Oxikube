//! The `resource::ApplyPlan` handler: the plan rebuilt from the command's objects, then the dry-run
//! pass (a dry-run dispatch stops there) or the real pass.

use oxikube_domain::OxiError;
use oxikube_domain::OxiResult;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::manifests::{ManifestDoc, SourceId};
use serde_json::json;

use super::build::build_plan;
use super::item::Item;
use super::run::Run;
use super::types::PlanReport;
use crate::apply::handler::DEFAULT_FIELD_MANAGER;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use crate::session::ClusterSessionManager;
use crate::workloads::new_batch_id;

/// How many objects one plan takes: a directory of a few hundred manifests fits many times over,
/// and a bigger one is refused before anything is sent.
pub const MAX_PLAN_OBJECTS: usize = 5_000;

/// The label the command's objects are read under (they arrive parsed already).
const SOURCE: &str = "resource::ApplyPlan";

/// Registers the `resource::ApplyPlan` handler on `registry`.
pub(crate) fn register(
    registry: &mut CommandRegistry,
    sessions: ClusterSessionManager,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::RESOURCE_APPLY_PLAN)
        .ok_or(RegisterError::Undeclared(CommandId::RESOURCE_APPLY_PLAN))?;
    registry.register(meta, move |command: Command, cx: HandlerContext| {
        let sessions = sessions.clone();
        async move { handle(command, cx, sessions).await }
    })
}

async fn handle(
    command: Command,
    cx: HandlerContext,
    sessions: ClusterSessionManager,
) -> OxiResult<CommandOutput> {
    let Command::ResourceApplyPlan {
        cluster,
        namespace,
        objects,
        field_manager,
        force_conflicts,
        stop_on_error,
    } = command
    else {
        return Err(OxiError::validation("not a resource::ApplyPlan command"));
    };
    let mutation = cx.require_mutation()?;
    if objects.is_empty() {
        return Err(OxiError::validation("the plan has no objects"));
    }
    if objects.len() > MAX_PLAN_OBJECTS {
        return Err(OxiError::validation(format!(
            "the plan has {} objects; one apply takes at most {MAX_PLAN_OBJECTS}",
            objects.len()
        )));
    }
    let session = sessions
        .get(&cluster)
        .ok_or_else(|| OxiError::not_found("the cluster is not open"))?;
    let reader = session
        .resources()
        .ok_or_else(|| OxiError::conflict("the cluster is not connected"))?;
    let discovery = session.discovery();
    let docs: Vec<ManifestDoc> = objects
        .0
        .into_iter()
        .enumerate()
        .map(|(index, json)| ManifestDoc {
            source: SourceId::new(SOURCE),
            line: index + 1,
            json,
        })
        .collect();
    let plan = build_plan(docs, namespace.as_deref(), discovery.as_deref()).await;
    let manager = field_manager
        .filter(|manager| !manager.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_FIELD_MANAGER.to_owned());
    let items = Item::all(plan, &cluster, &manager, force_conflicts);
    let batch = new_batch_id("apply");
    let clock = sessions.clock();
    let run = Run {
        cx: &cx,
        mutation,
        reader: reader.as_ref(),
        discovery: discovery.as_deref(),
        clock: clock.as_ref(),
        batch: &batch,
        items: &items,
        stop_on_error,
    };
    let objects = if run.dry_run() {
        run.dry_run_all().await
    } else {
        run.apply_all().await
    };
    let mut report = PlanReport { objects };
    report.sort();
    cx.note_audit(format!(
        "batch={batch} items={} {}",
        items.len(),
        counts(&report)
    ));
    let summary = report.summary();
    Ok(CommandOutput {
        message: Some(if run.dry_run() {
            format!("Dry run: {summary}")
        } else {
            summary
        }),
        data: Some(json!({
            "batch": batch,
            "dry_run": run.dry_run(),
            "objects": report.objects,
        })),
    })
}

/// `created=2 configured=0 ...`: the audit note's counts, names only.
fn counts(report: &PlanReport) -> String {
    [
        ("created", "created"),
        ("configured", "configured"),
        ("unchanged", "unchanged"),
        ("deferred", "deferred"),
        ("no_preview", "no preview"),
        ("failed", "failed"),
        ("skipped", "skipped"),
    ]
    .iter()
    .map(|(key, word)| format!("{key}={}", report.count(word)))
    .collect::<Vec<_>>()
    .join(" ")
}
