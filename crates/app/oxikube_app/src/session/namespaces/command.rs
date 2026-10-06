//! The `namespace::*` commands: the handler a `CommandBus` registers (non-negotiable 4).
//!
//! The palette, the keymap, the selector's buttons and the MCP tools
//! `app.namespace_select` and `app.namespace_toggle_favourite` all arrive as a [`Command`] and
//! run here. Both change the view only (read-only commands, no `MutationGuard`): the selection
//! picks which namespaces are watched, the favourites are local state.

use oxikube_domain::command::Command;
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::{OxiError, OxiResult};

use super::service::{NamespaceOutcome, NamespaceService};

impl NamespaceService {
    /// Runs a `namespace::Select` or `namespace::ToggleFavourite` command.
    ///
    /// # Errors
    ///
    /// `Unsupported` for any other command; otherwise as
    /// [`select`](NamespaceService::select) and
    /// [`toggle_favourite`](NamespaceService::toggle_favourite).
    pub async fn execute(&self, command: &Command) -> OxiResult<NamespaceOutcome> {
        match command {
            Command::NamespaceSelect {
                cluster,
                namespaces,
            } => {
                self.select(cluster, NamespaceSelection::from_names(namespaces))
                    .await
            }
            Command::NamespaceToggleFavourite { cluster, namespace } => {
                self.toggle_favourite(cluster, namespace).await
            }
            other => Err(OxiError::unsupported(format!(
                "{} is not a namespace command",
                other.id()
            ))),
        }
    }
}
