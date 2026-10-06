//! What the selector tells its host, and the toast the host shows for it.

use gpui::SharedString;
use oxikube_workspace::Toast;

/// Emitted by the [`NamespaceSelector`](super::NamespaceSelector). The host (the cluster tab)
/// shows a toast for each through `Workspace::show_toast`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamespaceSelectorEvent {
    /// Namespaces remembered for this cluster no longer exist and were dropped from the
    /// selection. Show [`stale_dropped_toast`].
    StaleDropped(Vec<String>),
    /// A change could not be applied or remembered; the selector shows the namespaces the
    /// session really has again. The text is already redacted (it comes from an `OxiError`).
    Failed(SharedString),
}

/// The toast for [`NamespaceSelectorEvent::StaleDropped`]: `Namespace "gone" no longer exists
/// and was removed from the selection.`
pub fn stale_dropped_toast(names: &[String]) -> Toast {
    let quoted: Vec<String> = names.iter().map(|n| format!("\"{n}\"")).collect();
    let message = match quoted.as_slice() {
        [one] => format!("Namespace {one} no longer exists and was removed from the selection."),
        many => format!(
            "Namespaces {} no longer exist and were removed from the selection.",
            many.join(", ")
        ),
    };
    Toast::info(message).key("namespace-selector-stale")
}
