//! The `kubeconfig::*` commands: the handlers a `CommandBus` registers (non-negotiable 4).
//!
//! The sources screen's buttons, the palette and the MCP tools `app.kubeconfig_add_source`,
//! `app.kubeconfig_remove_source` and `app.kubeconfig_reload` all arrive as a [`Command`] and
//! run here. None of them changes a cluster, so none goes through `MutationGuard`: they change
//! the user's settings list and Oxikube's own files. The UI asks before it removes a stored
//! kubeconfig; for the other initiators the rule is in the handler: an agent or an extension
//! cannot delete a stored kubeconfig, and no output ever carries credentials (rows hold paths,
//! counts and fixed-wording notes).

use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{self, Command, CommandId, KubeconfigSourceRef};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{SourceState, UserSource, UserSourceKind};
use serde_json::{Value, json};

use super::row::SourceRow;
use super::service::{KubeconfigSourcesService, SourceChange};
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};

/// Registers the three `kubeconfig::*` commands, with their tool stubs, over `service`.
///
/// Call it through [`CommandRegistry::install`] from the crate that owns the screen.
///
/// # Errors
///
/// A [`RegisterError`] when one of the ids is registered already.
pub fn register_commands(
    registry: &mut CommandRegistry,
    service: &KubeconfigSourcesService,
) -> Result<(), RegisterError> {
    for id in [
        CommandId::KUBECONFIG_ADD_SOURCE,
        CommandId::KUBECONFIG_REMOVE_SOURCE,
        CommandId::KUBECONFIG_RELOAD,
    ] {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let service = service.clone();
        registry.register(meta, move |command: Command, cx: HandlerContext| {
            let service = service.clone();
            let initiator = cx.initiator();
            async move { service.execute(&command, initiator).await }
        })?;
    }
    Ok(())
}

impl KubeconfigSourcesService {
    /// Runs a `kubeconfig::*` command for `initiator`.
    ///
    /// # Errors
    ///
    /// `Unsupported` for any other command; `Forbidden` when an agent or plugin removes a
    /// kubeconfig Oxikube stored itself; otherwise as [`add`](Self::add),
    /// [`remove`](Self::remove) and [`reload`](Self::reload).
    pub async fn execute(
        &self,
        command: &Command,
        initiator: Initiator,
    ) -> OxiResult<CommandOutput> {
        let message = match command {
            Command::KubeconfigAddSource { source } => {
                let change = self.add(source).await?;
                added_message(&change)
            }
            Command::KubeconfigRemoveSource { source } => {
                if matches!(initiator, Initiator::Agent | Initiator::Plugin)
                    && self.removal_deletes_file(source)
                {
                    return Err(OxiError::forbidden(
                        "Only the user can delete a kubeconfig that Oxikube stored.",
                    ));
                }
                let change = self.remove(source).await?;
                match change.deleted_file {
                    Some(_) => "Removed the source and deleted its stored kubeconfig.".to_owned(),
                    None => "Removed the source. The file itself was not touched.".to_owned(),
                }
            }
            Command::KubeconfigReload => {
                let changed = self.reload().await?;
                format!(
                    "Reloaded kubeconfigs: {} added, {} removed, {} changed.",
                    changed.added.len(),
                    changed.removed.len(),
                    changed.changed.len()
                )
            }
            other => {
                return Err(OxiError::unsupported(format!(
                    "{} is not a kubeconfig command",
                    other.id()
                )));
            }
        };
        let rows = self.rows().await?;
        Ok(CommandOutput {
            message: Some(message),
            data: Some(json!({ "sources": rows.iter().map(row_json).collect::<Vec<_>>() })),
        })
    }

    /// Whether removing `which` would delete a stored file.
    fn removal_deletes_file(&self, which: &KubeconfigSourceRef) -> bool {
        match which {
            KubeconfigSourceRef::File { path } => {
                self.deletes_file_on_remove(&UserSource::file(path.trim()))
            }
            _ => false,
        }
    }
}

fn added_message(change: &SourceChange) -> String {
    match (&change.created_file, change.unchanged) {
        (Some(_), _) => format!(
            "Stored the pasted kubeconfig ({} context{}).",
            change.contexts_in_paste,
            if change.contexts_in_paste == 1 {
                ""
            } else {
                "s"
            }
        ),
        (None, true) => "That source is already in the list.".to_owned(),
        (None, false) => "Added the source.".to_owned(),
    }
}

/// A row as JSON for tool output: no credentials, only what the screen shows.
fn row_json(row: &SourceRow) -> Value {
    let kind = match row.source.kind {
        UserSourceKind::Default => "default",
        UserSourceKind::File => "file",
        UserSourceKind::Dir => "dir",
    };
    let state = row.state.map(|state| match state {
        SourceState::Found => "found",
        SourceState::Blank => "blank",
        SourceState::Missing => "missing",
        SourceState::Unreadable => "unreadable",
        SourceState::Invalid => "invalid",
    });
    json!({
        "kind": kind,
        "path": row.source.path.as_ref().map(|p| p.display().to_string()),
        "state": state,
        "contexts": row.contexts,
        "message": row.message,
        "stored_by_oxikube": row.stored,
    })
}
