//! `pod::Debug` (E09-S10): add an ephemeral debug container to a pod, then open a terminal in it.
//!
//! The command is a mutation, so the bus's guard has checked read-only mode, had the user confirm
//! and written the audit record's start before this handler runs; the handler holds the guard's
//! permit and spends it on [`ExecService::open_debug`], which patches the pod, waits for the
//! container to run and attaches. Only then does it queue the terminal: a plain pod-attach
//! terminal ([`BackendDescriptor::Attach`]) of the new container, which claims the session that was
//! just opened. A failure (the API server's refusal, a timeout) is the command's error, shown by
//! whoever dispatched it and recorded as `Failed` in the audit log.

use std::sync::Arc;

use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_app::{DebugRequest, ExecService};
use oxikube_domain::OxiResult;
use oxikube_domain::command::{self, Command, CommandId};
use serde_json::json;

use super::BackendDescriptor;
use super::commands::{TerminalRequest, TerminalViewSink, ensure_pod};

/// Registers `pod::Debug` on `registry` (with its MCP tool stub: unsafe, interactive, hidden from
/// agents by default). Call it with the same sink as `register_pod_commands`, and the app's one
/// `ExecService`.
///
/// # Errors
///
/// A [`RegisterError`] when the id is registered twice (a wiring bug).
pub fn register_debug_command(
    registry: &mut CommandRegistry,
    sink: TerminalViewSink,
    exec: Arc<ExecService>,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::POD_DEBUG)
        .ok_or(RegisterError::Undeclared(CommandId::POD_DEBUG))?;
    registry.register(meta, move |command: Command, cx: HandlerContext| {
        let (sink, exec) = (sink.clone(), exec.clone());
        async move { debug(&sink, &exec, command, &cx).await }
    })
}

async fn debug(
    sink: &TerminalViewSink,
    exec: &ExecService,
    command: Command,
    cx: &HandlerContext,
) -> OxiResult<CommandOutput> {
    let request = DebugRequest::from_command(&command)?;
    ensure_pod(&request.pod, "pod::Debug")?;
    let opened = exec.open_debug(cx.require_mutation()?, &request).await?;
    let plan = opened.plan;
    let data = json!({
        "container": plan.name,
        "image": plan.image,
        "target": &*plan.target,
        "dry_run": opened.dry_run,
    });
    if opened.dry_run {
        return Ok(CommandOutput {
            message: Some(format!(
                "Dry run: a debug container running {} can be added to {}",
                plan.image, request.pod.name
            )),
            data: Some(data),
        });
    }
    let descriptor = BackendDescriptor::Attach {
        pod: request.pod.clone(),
        container: Some(plan.name.clone()),
    };
    if let Err(error) = sink.send(TerminalRequest::Pod(descriptor)) {
        // Nobody will claim the session: end it. The container stays in the pod.
        exec.discard_debug(&request.pod, &plan.name);
        return Err(error);
    }
    Ok(CommandOutput {
        message: Some(format!(
            "Debug container {} is running in {}",
            plan.name, request.pod.name
        )),
        data: Some(data),
    })
}
