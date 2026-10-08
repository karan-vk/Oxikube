//! What a row key says when it cannot do what it was asked (E07-U563): a short toast instead of
//! silence, and the note that Shell, Attach and Debug act on the cursor row of a selection.

use oxikube_domain::command::CommandId;
use oxikube_domain::ids::ResourceRef;
use oxikube_workspace::Toast;

use super::view::ResourceTable;
use crate::actions::ResourceActions;
use crate::exec::{acts_on_cursor_row, availability_hint};

impl ResourceTable {
    /// The one object an exec-class `command` acts on when `targets` holds several: the cursor
    /// row (else the first target). `None` when `command` takes a selection or `targets` is one
    /// object.
    pub(super) fn cursor_target(
        &self,
        command: CommandId,
        targets: &[ResourceRef],
        cx: &gpui::App,
    ) -> Option<ResourceRef> {
        if targets.len() < 2 || !acts_on_cursor_row(command) {
            return None;
        }
        self.cursor_row(cx)
            .and_then(|row| self.row_ref(row, cx))
            .or_else(|| targets.first().cloned())
    }

    /// Says that `label` runs on the cursor row `target` only, though `selected` rows are selected.
    pub(super) fn say_cursor_row(
        &self,
        command: CommandId,
        label: &str,
        target: &ResourceRef,
        selected: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        self.toast(
            Toast::info(format!(
                "{label} acts on the cursor row ({}), not the {selected} selected",
                target.name
            ))
            .key(format!("action-cursor:{command}")),
            cx,
        );
    }

    /// Says why `command` did not run: no row, a kind without the action, a selection of several
    /// for a one-object action, or rights the session was never granted.
    pub(super) fn say_unavailable(
        &self,
        actions: &ResourceActions,
        command: CommandId,
        selected: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let message = self.unavailable_reason(actions, command, selected);
        self.toast(
            Toast::warning(message).key(format!("action-unavailable:{command}")),
            cx,
        );
    }

    fn unavailable_reason(
        &self,
        actions: &ResourceActions,
        command: CommandId,
        selected: usize,
    ) -> String {
        let kind = &self.kind;
        let label = short_label(command);
        if actions.offers(kind, command) {
            if selected > 1 {
                format!("{label} works on one object at a time; select a single row")
            } else {
                format!("{label} needs rights this cluster has not granted you")
            }
        } else if let Some(hint) = availability_hint(command) {
            hint.to_owned()
        } else {
            format!("{label} is not available for {}", kind.gvk.kind)
        }
    }

    /// Says that no row is there to act on.
    pub(super) fn say_no_row(&self, cx: &mut gpui::Context<Self>) {
        self.toast(Toast::info("Select a row first").key("action-no-row"), cx);
    }
}

/// The short name of a row action for a toast ("Shell", not "Node Shell" or "Delete Resource").
fn short_label(command: CommandId) -> &'static str {
    match command {
        CommandId::POD_SHELL | CommandId::NODE_SHELL => "Shell",
        CommandId::RESOURCE_DELETE => "Delete",
        other => oxikube_domain::command::lookup(other).map_or("This action", |meta| meta.title),
    }
}
