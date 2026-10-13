//! The row actions of the detail views (E11-S07): "View YAML" and "Describe" on any row, the
//! context-menu and palette counterparts of k9s's `y` and `d`.
//!
//! The binary registers them with the app's
//! [`RowActionRegistry`](oxikube_app::RowActionRegistry) next to the others. Both only open the
//! detail drawer on a tab, so they change nothing in a cluster and stay available on a read-only
//! one.

use oxikube_app::actions::KindFilter;
use oxikube_app::{RowActionSpec, SubjectKind};
use oxikube_domain::command::{Command, CommandId};

/// Where "View YAML" sits in a row's menu: first, before "Describe" and the per-kind actions.
const VIEW_YAML_ORDER: u16 = 10;
/// Where "Describe" sits: after "View YAML".
const VIEW_DESCRIBE_ORDER: u16 = 20;

/// Where "View Events" sits (E14-S04): after "Describe".
const VIEW_EVENTS_ORDER: u16 = 25;

/// Where "View Metrics" sits: after "View Events".
const VIEW_METRICS_ORDER: u16 = 30;

/// The row actions this module adds: `resource::ViewYaml`, `resource::ViewDescribe` and
/// `resource::ViewEvents`, for every kind, and `resource::ViewMetrics` for the kinds that have history graphs (pods, nodes,
/// workloads, namespaces, PVCs, ingresses), one object at a time.
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
        RowActionSpec::new(CommandId::RESOURCE_VIEW_EVENTS, |target| {
            Command::ResourceViewEvents {
                target: target.clone(),
            }
        })
        .label("View Events")
        .order(VIEW_EVENTS_ORDER),
        RowActionSpec::new(CommandId::RESOURCE_VIEW_METRICS, |target| {
            Command::ResourceViewMetrics {
                target: target.clone(),
            }
        })
        .label("View Metrics")
        .kinds(KindFilter::Matching(|kind| {
            SubjectKind::of(&kind.gvk).is_some()
        }))
        .order(VIEW_METRICS_ORDER),
    ]
}
