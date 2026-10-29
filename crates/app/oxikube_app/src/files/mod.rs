//! The pod file browser's app half (E20-S07): [`FileService`] lists a container's directories,
//! downloads files and folders and uploads files, over the exec streams of the session
//! ([`ExecTransferExt`](oxikube_ports::ExecTransferExt), E20-S06).
//!
//! | Piece | Where |
//! |---|---|
//! | the `ls -la` / `find` parsers, pure, with a fixture corpus | `parse` |
//! | [`FileEntry`], [`Listing`], path helpers | `entry` |
//! | one-shot command capture over the exec stream | `capture` |
//! | listing with the `--full-time` and `find` fallbacks | `list` |
//! | [`FileService`]: open, list, exists, the last directory per pod | `service` |
//! | downloading a file or folder | `download` |
//! | uploading a file behind the guard's permit | `upload` |
//! | [`TransferBoard`]: the transfers of this run, for the panel | `board` |
//! | the `pod::DownloadFile` / `pod::UploadFile` handlers | `commands` |
//! | [`FileRunner`]: what a view calls to run them through the bus | `runner` |
//!
//! # Policy
//!
//! * **Browsing and downloading are reads** (`pod::BrowseFiles`, `pod::DownloadFile`): no
//!   confirmation, no audit record, allowed on a read-only cluster (the cluster's RBAC decides
//!   whether the account may exec). They run `ls`, `find`, `cat` or `tar` in the container, and
//!   write only the local path the user chose. Because a download writes the user's disk and is
//!   not confirmed, only the app's own UI and the palette may start one (an agent or a plugin gets
//!   `Forbidden`).
//! * **Upload mutates the pod** (`pod::UploadFile`): a guarded command, blocked on a read-only
//!   cluster, low risk for a new file and medium when it replaces one (a simple confirmation
//!   either way), audited once with the container, destination and size. The handler gets the
//!   guard's [`Mutation`](crate::Mutation); the stream-level exec port is not handed to views.
//! * **Nothing is logged or kept**: file names are shown, never contents; the board holds names
//!   and byte counts for this run; the last directory per pod is memory only. Browsing
//!   `/var/run/secrets` shows a one-line warning in the view ([`is_secret_mount`]).
//!
//! # Listing
//!
//! `ls -la` with a fixed locale, `--full-time` where the container's `ls` has it; a container
//! without `ls` gets `find` (names and kinds); one with no shell tools gets a clear
//! [`NO_SHELL_TOOLS`] message instead of an error loop. A symbolic link is opened as a directory
//! by listing `path/`.

mod board;
mod capture;
mod commands;
mod download;
mod entry;
mod list;
mod parse;
mod runner;
mod service;
#[cfg(test)]
mod tests;
mod upload;

pub use board::{
    Direction, SettleOnDrop, TransferBoard, TransferHandle, TransferId, TransferInfo,
    TransferStart, TransferState,
};
pub use commands::{audit_detail, register_commands, summary};
pub use download::{DownloadOutcome, DownloadRequest};
pub use entry::{FileEntry, FileKind, Listing, ListingSource, directory_argument, join, parent_of};
pub use list::NO_SHELL_TOOLS;
pub use parse::{ParsedLs, parse_find, parse_ls};
pub use runner::FileRunner;
pub use service::{FileService, OpenedPod};
pub use upload::{UploadPlan, UploadRequest};

/// Whether `path` is under a mounted-secrets directory (service account tokens, projected
/// secrets): the view warns before showing or downloading from it.
pub fn is_secret_mount(path: &str) -> bool {
    ["/var/run/secrets", "/run/secrets", "/var/secrets"]
        .iter()
        .any(|root| path == *root || path.starts_with(&format!("{root}/")))
}
