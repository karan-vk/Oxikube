//! The row action of any resource: "Copy to…".

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::{Command, CommandId};

/// Where "Copy to…" sits in a menu: after the kind's own actions, before the generic Delete.
pub const COPY_TO_ORDER: u16 = 800;

/// The row actions this module adds: `resource::CopyTo` for every kind, one object at a time.
///
/// It needs the session's `MUTATE` grant, so a read-only *source* greys it out too; reading from
/// a read-only cluster is allowed by the guard, but the action is the command's row and a table of
/// a read-only cluster offers none of the mutating ones. (The target's read-only flag is checked
/// when it is chosen.) The command built here names no target: the table and the detail panel
/// open the [`CopyDialog`](super::CopyDialog) and send the target the user chose.
pub fn copy_row_actions() -> Vec<RowActionSpec> {
    vec![
        RowActionSpec::new(CommandId::RESOURCE_COPY_TO, |source| {
            Command::ResourceCopyTo {
                source: source.clone(),
                target: source.clone(),
                overwrite: false,
                force: false,
            }
        })
        .label("Copy to…")
        .kinds(KindFilter::Any)
        .order(COPY_TO_ORDER),
    ]
}
