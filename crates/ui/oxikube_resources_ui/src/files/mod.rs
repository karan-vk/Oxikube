//! The pod file browser (E20-S07): a workspace item that lists a container's directories, with
//! size, mode and time, and downloads and uploads files with progress and cancel.
//!
//! | Piece | Where |
//! |---|---|
//! | [`FilesView`]: the item (toolbar, breadcrumb, virtualised list, transfers panel, drop target) | `view` |
//! | [`FilesModel`]: entries sorted (directories first), hidden-file filter, cell text | `model`, `format` |
//! | `pod::BrowseFiles`: [`register_commands`], [`FilesCommandSink`], [`FilesViews`] | `commands` |
//! | [`files_row_actions`]: "Browse Files" on a pod's row | here |
//!
//! # Where the logic is
//!
//! The listing, the parsers and the transfers are `oxikube_app::files` (plain async Rust over the
//! ports, tested with fakes); this module is the view over them. Downloads and uploads go through
//! the command bus (`pod::DownloadFile`, `pod::UploadFile`), so the guard's read-only block,
//! confirmation and audit apply to an upload however it starts: the toolbar's button, the row
//! menu or a drop from the OS.
//!
//! # How a user gets here
//!
//! "Browse Files" in a pod row's menu and detail (and the palette's `pod::BrowseFiles`) open the
//! pod's browser as a tab of the cluster. For a pod with several containers the toolbar has one
//! button per container.

mod commands;
mod format;
mod model;
#[cfg(test)]
mod tests;
mod view;

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::{Command, CommandId};

pub use commands::{
    FilesCommandSink, FilesHost, FilesRequest, FilesViews, FilesViewsDeps, register_commands,
};
pub use format::{entry_size_text, mode_text, size_text, time_text};
pub use model::{FileColumn, FileRow, FileSort, FilesModel, PreparedListing};
pub use view::{FilesDelegate, FilesView, FilesViewDeps, LoadState, item_key};

/// Where "Browse Files" sits in a pod's menu: after "Debug" (130).
pub const BROWSE_FILES_ORDER: u16 = 140;

/// The row action this module adds: `pod::BrowseFiles` for pods. It reads, so it is offered on a
/// read-only cluster too; the cluster's RBAC decides whether the account may exec.
pub fn files_row_actions() -> Vec<RowActionSpec> {
    vec![
        RowActionSpec::new(CommandId::POD_BROWSE_FILES, |target| {
            Command::PodBrowseFiles {
                target: target.clone(),
                container: None,
                path: None,
            }
        })
        .label("Browse Files")
        .kinds(KindFilter::Matching(|kind| kind.gvk.is_pod()))
        .order(BROWSE_FILES_ORDER),
    ]
}
