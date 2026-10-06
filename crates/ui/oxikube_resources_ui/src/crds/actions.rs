//! The row actions of the CRD list: open the custom resources a definition defines, and show
//! its details.

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::{Command, CommandId};

use super::info::is_crd_kind;

/// The specs [`ResourceActions`](crate::actions::ResourceActions) adds to the core registry: for
/// CustomResourceDefinition rows only, "Open Custom Resources" (`crd::OpenResources`, what Enter
/// and a double click do on a CRD row) and "Show Details" (`resource::Open`, the drawer with the
/// schema). Both read; neither needs the guard.
pub fn crd_row_actions() -> [RowActionSpec; 2] {
    let crd = KindFilter::Matching(|kind| is_crd_kind(&kind.gvk));
    [
        RowActionSpec::new(CommandId::CRD_OPEN_RESOURCES, |target| {
            Command::CrdOpenResources {
                cluster: target.cluster.clone(),
                name: target.name.to_string(),
            }
        })
        .label("Open Custom Resources")
        .kinds(crd)
        .order(10),
        RowActionSpec::new(CommandId::RESOURCE_OPEN, |target| Command::ResourceOpen {
            target: target.clone(),
        })
        .label("Show Details")
        .kinds(crd)
        .order(20),
    ]
}
