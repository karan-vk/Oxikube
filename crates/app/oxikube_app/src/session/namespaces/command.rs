//! The `namespace::*` commands: the handler a `CommandBus` registers (non-negotiable 4).
//!
//! The palette, the keymap, the selector's buttons and the MCP tools
//! `app.namespace_select` and `app.namespace_toggle_favourite` all arrive as a [`Command`] and
//! run here. Both change the view only (read-only commands, no `MutationGuard`): the selection
//! picks which namespaces are watched, the favourites are local state.
//!
//! `namespace::Select` is an immediate command on the bus (E05-P600): [`execute_now`](
//! NamespaceService::execute_now) sets the session's selection on the caller's thread and
//! returns the remembering, so the UI shows the new selection in the frame after the input.

use oxikube_domain::command::Command;
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::{OxiError, OxiResult};

use super::now::SelectedNow;
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

    /// Runs a `namespace::Select` command now ([`select_now`](Self::select_now)); the
    /// returned [`SelectedNow`] remembers it.
    ///
    /// # Errors
    ///
    /// `Unsupported` for any other command; otherwise as [`select_now`](Self::select_now).
    pub fn execute_now(&self, command: &Command) -> OxiResult<SelectedNow> {
        match command {
            Command::NamespaceSelect {
                cluster,
                namespaces,
            } => self.select_now(cluster, NamespaceSelection::from_names(namespaces)),
            other => Err(OxiError::unsupported(format!(
                "{} is not an immediate namespace command",
                other.id()
            ))),
        }
    }
}
