//! The row actions of a Namespace: use it, pin it, delete it.
//!
//! Each is a command (palette, key and menu send the same one):
//!
//! | Action | Command | Key |
//! |---|---|---|
//! | Use namespace | `namespace::Select` with this one name | `u` |
//! | All namespaces | `namespace::UseFavourite` with slot 0 | `0` |
//! | Toggle favourite | `namespace::ToggleFavourite` | `p` |
//! | New namespace… | `namespace::Create` (opens the create dialog) | toolbar **New** |
//! | Delete | `namespace::Delete` (type the name; warnings and counts first) | `ctrl-d`, `delete` |
//!
//! `0`-`9` (`namespace::UseFavourite`) is a table-wide key, not a row action.

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::ResourceRef;

/// Where "Use namespace" sits in a Namespace's menu: first (after "View YAML" and "Describe").
pub const USE_ORDER: u16 = 100;
/// Where "All namespaces" sits: after "Use namespace".
pub const ALL_ORDER: u16 = 105;
/// Where "Toggle favourite" sits: after "All namespaces".
pub const FAVOURITE_ORDER: u16 = 110;
/// Where "New namespace" sits: after "Toggle favourite".
pub const CREATE_ORDER: u16 = 120;
/// Where "Delete" sits: last, as the generic delete does.
pub const DELETE_ORDER: u16 = 900;

/// `namespace::Select` with this one namespace: "use this namespace" (k9s `u`).
pub(super) fn use_command(target: &ResourceRef) -> Command {
    Command::NamespaceSelect {
        cluster: target.cluster.clone(),
        namespaces: vec![target.name.to_string()],
    }
}

/// The row actions of the Namespaces table. "Delete" is the only one that changes the cluster
/// (guarded: read-only mode disables it, the name is typed); "Use" and "Toggle favourite" change
/// what the session watches and what is pinned, so they stay available on a read-only cluster.
pub fn namespace_row_actions() -> Vec<RowActionSpec> {
    let namespaces = KindFilter::Matching(|kind| kind.gvk.is_namespace());
    vec![
        RowActionSpec::new(CommandId::NAMESPACE_SELECT, use_command)
            .label("Use namespace")
            .kinds(namespaces)
            .order(USE_ORDER),
        // The "All namespaces" choice (k9s's `0`): slot 0 of `namespace::UseFavourite`.
        RowActionSpec::new(CommandId::NAMESPACE_USE_FAVOURITE, |target| {
            Command::NamespaceUseFavourite {
                cluster: target.cluster.clone(),
                slot: 0,
            }
        })
        .label("All namespaces")
        .kinds(namespaces)
        .order(ALL_ORDER),
        RowActionSpec::new(CommandId::NAMESPACE_TOGGLE_FAVOURITE, |target| {
            Command::NamespaceToggleFavourite {
                cluster: target.cluster.clone(),
                namespace: target.name.to_string(),
            }
        })
        .label("Toggle favourite")
        .kinds(namespaces)
        .order(FAVOURITE_ORDER),
        // Not about the row: it opens the create dialog (the name and labels are asked there), so
        // the command built here is the empty one the dialog replaces.
        RowActionSpec::new(CommandId::NAMESPACE_CREATE, |target| {
            Command::NamespaceCreate {
                cluster: target.cluster.clone(),
                name: String::new(),
                labels: Default::default(),
            }
        })
        .label("New namespace…")
        .kinds(namespaces)
        .order(CREATE_ORDER),
        RowActionSpec::new(CommandId::NAMESPACE_DELETE, |target| {
            Command::NamespaceDelete {
                target: target.clone(),
                contents: Default::default(),
            }
        })
        .label("Delete")
        .kinds(namespaces)
        .bulk()
        .order(DELETE_ORDER),
    ]
}
