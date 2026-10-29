//! Pod file transfer (E20-S06): [`TransferService`] over the tar helpers of
//! [`oxikube_ports::exec::transfer`], and the local half: [`safe_unpack`] for a download,
//! [`tar_file_stream`] for an upload.
//!
//! | Piece | Where |
//! |---|---|
//! | [`TransferService`]: download (with the `cat` fallback for one file), guarded upload | `service` |
//! | [`safe_unpack`]: a tar stream onto disk through `FsPort`, refusing escapes | `unpack` |
//! | [`tar_file_stream`]: one local file as a tar stream, built as it is sent | `pack` |
//!
//! # Policy
//!
//! A download is an exec in the container and reads only: the command that leads here (the file
//! browser, E20-S07) is exec-class, blocked on a read-only cluster unless `exec_in_read_only`
//! allows it, and audited. An upload changes the pod's filesystem: [`TransferService::upload`]
//! takes a [`Mutation`](crate::Mutation), which only `MutationGuard` creates, and the session's
//! stream-level exec port is crate-private
//! ([`ClusterSession`](crate::ClusterSession) does not hand it out), so no view can reach the
//! upload around the guard. Its command's audit detail carries the container, path and byte
//! count, never the content.
//!
//! Nothing here logs file content or what the remote printed.

mod pack;
mod service;
#[cfg(test)]
mod tests;
mod unpack;

pub use pack::tar_file_stream;
pub use service::{Download, DownloadFormat, RemoteEntry, TransferService, UploadReceipt};
pub use unpack::{Skipped, UnpackReport, safe_unpack};
