//! The row actions that open the editor on an object: "Edit" (`resource::Edit`, also k9s's `e`)
//! and "View YAML in Editor" (`resource::ViewYamlInEditor`), in a row's context menu and in the
//! palette's list for the selection. The binary registers them with the app's
//! [`RowActionRegistry`](oxikube_app::RowActionRegistry) next to the others.
//!
//! Both only read the object and open a tab, so neither is a mutation and both stay available on
//! a read-only cluster (an edit tab then opens read-only, with a notice; applying is
//! `resource::Apply`, which is guarded).

use oxikube_app::RowActionSpec;
use oxikube_domain::command::{Command, CommandId};

/// Where "View YAML in Editor" sits in a row's menu: after "View YAML" (10).
pub const VIEW_ORDER: u16 = 15;
/// Where "Edit" sits: after "Describe" (20), before the per-kind actions.
pub const EDIT_ORDER: u16 = 30;

/// Where "Diff With…" sits: right after "Edit".
pub const DIFF_ORDER: u16 = 31;

/// The row actions this module adds, for every kind, one object at a time.
pub fn editor_row_actions() -> Vec<RowActionSpec> {
    vec![
        RowActionSpec::new(CommandId::RESOURCE_VIEW_YAML_IN_EDITOR, |target| {
            Command::ResourceViewYamlInEditor {
                target: target.clone(),
            }
        })
        .label("View YAML in Editor")
        .order(VIEW_ORDER),
        RowActionSpec::new(CommandId::RESOURCE_EDIT, |target| Command::ResourceEdit {
            target: target.clone(),
        })
        .label("Edit")
        .order(EDIT_ORDER),
        // E20-S09: compare with the same kind in another cluster or namespace. It reads two
        // objects and writes nothing; without a second object the picker asks for it.
        RowActionSpec::new(CommandId::RESOURCE_DIFF_WITH, |target| {
            Command::ResourceDiffWith {
                target: target.clone(),
                other: None,
                include_status: false,
                compare_identity: false,
            }
        })
        .label("Diff With…")
        .order(DIFF_ORDER),
    ]
}
