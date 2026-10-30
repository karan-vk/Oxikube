//! The `resource::DiffWith` handler: reads both objects through the [`DiffService`] and hands the
//! result to the window's diff tab, or returns the changed-path summary to an agent.
//!
//! Read-only: no `MutationGuard` tier, nothing written. An agent (`k8s.resource_diff_with`) gets
//! the summary as text and structured data and no tab opens; a user gets the tab, which shows
//! progress while a cluster connects, then the diff. Nothing returned or sent carries an object
//! or a Secret value.

use std::sync::Arc;

use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_app::diff::{DiffProgress, DiffRequest, DiffService};
use oxikube_app::offload::off_io_workers;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::safety::Initiator;
use oxikube_domain::{OxiError, OxiResult};

use super::data::{DiffData, DiffState};
use crate::view::commands::{EditorRequest, EditorViewSink};

/// Registers `resource::DiffWith` on `registry`. Call it where the editor's commands are
/// registered: `registry.install("oxikube_editor::diff", |r| register_diff_command(r, sink, service))`.
///
/// # Errors
///
/// A [`RegisterError`] when the command is registered already.
pub fn register_diff_command(
    registry: &mut CommandRegistry,
    sink: EditorViewSink,
    service: DiffService,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::RESOURCE_DIFF_WITH)
        .ok_or(RegisterError::Undeclared(CommandId::RESOURCE_DIFF_WITH))?;
    registry.register(meta, move |command: Command, cx: HandlerContext| {
        let (sink, service) = (sink.clone(), service.clone());
        async move { handle(command, cx.initiator(), sink, service).await }
    })
}

async fn handle(
    command: Command,
    initiator: Initiator,
    sink: EditorViewSink,
    service: DiffService,
) -> OxiResult<CommandOutput> {
    let Command::ResourceDiffWith {
        target,
        other,
        include_status,
        compare_identity,
    } = command
    else {
        return Err(OxiError::validation("not a resource::DiffWith command"));
    };
    let shows_tab = matches!(initiator, Initiator::Ui | Initiator::Command);
    let Some(other) = other else {
        if !shows_tab {
            return Err(OxiError::validation(
                "resource::DiffWith needs `other`: the object to compare with",
            ));
        }
        sink.send(EditorRequest::PickDiffTarget { target })?;
        return Ok(CommandOutput::none());
    };
    let request = DiffRequest {
        left: target,
        right: other,
        include_status,
        compare_identity,
    };
    let show = |state: DiffState| {
        if shows_tab {
            // A closed window is no reason to fail the comparison.
            sink.send(EditorRequest::ShowResourceDiff {
                request: request.clone(),
                state,
            })
            .ok();
        }
    };
    show(DiffState::Working("Reading both objects…".to_owned()));
    let result = service
        .diff(&request, &|step: DiffProgress| {
            show(DiffState::Working(step.text()))
        })
        .await;
    let result = match result {
        Ok(diff) if shows_tab => {
            // Writing both sides as YAML and the line diff is CPU work (a 5 MB object takes a
            // while): on the runtime's blocking pool, never on an I/O worker.
            off_io_workers(move || DiffData::new(diff))
                .await
                .map(|data| {
                    show(DiffState::Ready(Arc::new(data)));
                    CommandOutput::none()
                })
        }
        Ok(diff) => Ok(CommandOutput {
            message: Some(diff.summary_text()),
            data: Some(diff.summary_data()),
        }),
        Err(error) => Err(error),
    };
    if let Err(error) = &result {
        show(DiffState::Failed(error.message().to_owned()));
    }
    result
}
