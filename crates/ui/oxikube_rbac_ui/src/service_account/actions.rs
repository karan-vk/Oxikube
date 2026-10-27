//! The extension point of the panel's header: buttons other stories add (E17-S09's "Generate
//! kubeconfig") without this module knowing about them.

use gpui::{App, Global};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;

/// One button in the ServiceAccount panel's header. Its click sends the command `command` builds
/// for the account through the command bus, like every other action: the guard confirms, audits
/// and refuses on a read-only cluster, so a header action never calls a port itself.
#[derive(Debug, Clone, Copy)]
pub struct HeaderAction {
    /// A stable id: the element id and the registry key (registering it twice replaces it).
    pub id: &'static str,
    /// The button's label.
    pub label: &'static str,
    /// The button's tooltip.
    pub tooltip: &'static str,
    /// The command to send for the account.
    pub command: fn(&ResourceRef) -> Command,
}

#[derive(Default)]
struct HeaderActions(Vec<HeaderAction>);

impl Global for HeaderActions {}

/// Adds `action` to the header of every ServiceAccount panel (replacing one with the same id).
pub fn register_header_action(cx: &mut App, action: HeaderAction) {
    let actions = &mut cx.default_global::<HeaderActions>().0;
    actions.retain(|existing| existing.id != action.id);
    actions.push(action);
}

/// The registered header actions, in registration order.
pub fn header_actions(cx: &App) -> Vec<HeaderAction> {
    cx.try_global::<HeaderActions>()
        .map(|actions| actions.0.clone())
        .unwrap_or_default()
}
