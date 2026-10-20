//! The start dialog's tests: the pure view-model, the error mapping and the ports read from an
//! object. The dialog as a view is tested where it is mounted (`oxikube_resources_ui`, the
//! Forward buttons, `shift-f` and the screenshots), over a real workspace.

mod form;
mod ports;
mod problem;
mod view;

use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};

pub(super) fn target(kind: &str, name: &str) -> ResourceRef {
    ResourceRef::new(
        ClusterId::new("kubeconfig", &ContextName::from("kind")),
        Gvk::new("", "v1", kind),
        Some("shop".into()),
        name,
    )
}
