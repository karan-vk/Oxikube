//! The `namespace::*` commands: the handler a `CommandBus` registers (non-negotiable 4).
//!
//! The palette, the keymap, the selector's buttons, the Namespaces table's `u` key and the MCP
//! tools `app.namespace_select`, `app.namespace_toggle_favourite` and
//! `app.namespace_use_favourite` all arrive as a [`Command`] and run here. They change the view
//! only (read-only commands, no `MutationGuard`): the selection picks which namespaces are
//! watched, the favourites are local state. "Use this namespace" (k9s `u`) is
//! `namespace::Select` with that one name; the digits `0`-`9` are `namespace::UseFavourite`.
//! Creating and deleting a namespace change the cluster and live in
//! [`crate::namespaces`].
//!
//! `namespace::Select` is an immediate command on the bus (E05-P600): [`execute_now`](
//! NamespaceService::execute_now) sets the session's selection on the caller's thread and
//! returns the remembering, so the UI shows the new selection in the frame after the input.

use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::{OxiError, OxiResult};

use super::now::SelectedNow;
use super::service::{NamespaceOutcome, NamespaceService};
use super::shortcuts::{MAX_SLOTS, slot_selection};

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
            Command::NamespaceUseFavourite { cluster, slot } => {
                self.use_favourite(cluster, *slot).await
            }
            other => Err(OxiError::unsupported(format!(
                "{} is not a namespace command",
                other.id()
            ))),
        }
    }

    /// Selects All (`slot` 0) or the `slot`-th favourite (1 to 9, k9s's number keys). A slot with
    /// no favourite changes nothing.
    ///
    /// # Errors
    ///
    /// `Validation` for a slot above 9; as [`select`](Self::select).
    pub async fn use_favourite(
        &self,
        cluster: &ClusterId,
        slot: u8,
    ) -> OxiResult<NamespaceOutcome> {
        check_slot(slot)?;
        self.ensure_loaded(cluster).await?;
        let prefs = self.cached(cluster);
        match slot_selection(&prefs.favourites, slot) {
            Some(selection) => self.select(cluster, selection).await,
            None => Ok(NamespaceOutcome {
                changed: false,
                prefs,
            }),
        }
    }

    /// [`use_favourite`](Self::use_favourite) for the frame of the key press: sets the session's
    /// selection now, on this thread, when the favourites are already loaded (they are once the
    /// selector has started for the cluster) or the slot is `0`. `Ok(None)` when it cannot say
    /// here: the favourites are not loaded yet, or no favourite sits at `slot`; the caller then
    /// runs [`use_favourite`](Self::use_favourite) off the UI thread.
    ///
    /// # Errors
    ///
    /// `Validation` for a slot above 9; `NotFound` when the session is not open.
    pub fn use_favourite_now(
        &self,
        cluster: &ClusterId,
        slot: u8,
    ) -> OxiResult<Option<SelectedNow>> {
        check_slot(slot)?;
        if slot == 0 {
            return self.select_now(cluster, NamespaceSelection::All).map(Some);
        }
        let selection = {
            let cache = self.shared.cache.lock();
            cache
                .get(cluster)
                .and_then(|prefs| slot_selection(&prefs.favourites, slot))
        };
        selection
            .map(|selection| self.select_now(cluster, selection))
            .transpose()
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

/// Slots are the digits `0`-`9`.
fn check_slot(slot: u8) -> OxiResult<()> {
    if usize::from(slot) > MAX_SLOTS {
        return Err(OxiError::validation(format!(
            "namespace slot {slot} is not a digit from 0 to {MAX_SLOTS}"
        )));
    }
    Ok(())
}
