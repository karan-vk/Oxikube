//! The row actions of the detail views (E11-S07): "View YAML" and "Describe" on any row, the
//! context-menu and palette counterparts of k9s's `y` and `d`.
//!
//! The binary registers them with the app's
//! [`RowActionRegistry`](oxikube_app::RowActionRegistry) next to the others. Both only open the
//! detail drawer on a tab, so they change nothing in a cluster and stay available on a read-only
//! one.

use oxikube_app::RowActionSpec;
use oxikube_domain::command::{Command, CommandId};

/// Where "View YAML" sits in a row's menu: first, before "Describe" and the per-kind actions.
const VIEW_YAML_ORDER: u16 = 10;
/// Where "Describe" sits: after "View YAML".
const VIEW_DESCRIBE_ORDER: u16 = 20;

/// The row actions this module adds: `resource::ViewYaml` and `resource::ViewDescribe`, for every
/// kind, one object at a time.
pub fn view_row_actions() -> Vec<RowActionSpec> {
    vec![
        RowActionSpec::new(CommandId::RESOURCE_VIEW_YAML, |target| {
            Command::ResourceViewYaml {
                target: target.clone(),
            }
        })
        .label("View YAML")
        .order(VIEW_YAML_ORDER),
        RowActionSpec::new(CommandId::RESOURCE_VIEW_DESCRIBE, |target| {
            Command::ResourceViewDescribe {
                target: target.clone(),
            }
        })
        .label("Describe")
        .order(VIEW_DESCRIBE_ORDER),
    ]
}
