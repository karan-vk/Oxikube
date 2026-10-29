//! File transfer over exec (E20-S06): `kubectl cp`'s tar streams as [`ExecTransferExt`] helpers
//! on every [`ExecStreamPort`](super::ExecStreamPort).
//!
//! | Piece | Where |
//! |---|---|
//! | [`ExecTransferExt`]: [`tar_download`](ExecTransferExt::tar_download), [`tar_upload`](ExecTransferExt::tar_upload), [`cat_download`](ExecTransferExt::cat_download) | `ext` |
//! | [`RemotePath`], [`TransferOptions`], the commands run in the container | `target` |
//! | [`TransferProgress`], [`ProgressFn`], the throttled meter | `progress` |
//! | [`CancellationToken`] (runtime-free) | `cancel` |
//! | the download stream (stdout as `Bytes`, stderr drained beside it) | `download` |
//! | the upload (archive to stdin, close, exit status) | `upload` |
//! | missing `tar`, missing path, no permission as typed errors | `failure` |
//!
//! # How it works
//!
//! A download runs `tar -cf - -C <parent> <name>` in the container with stdout and stderr on and
//! **no TTY** (a TTY rewrites bytes and would corrupt binary data); its stdout is the archive.
//! The API server does not stop an exec'd process when the connection closes, and a `tar`
//! blocked on output nobody reads would stay in the pod, so the download runs under a small `sh`
//! script that kills it when stdin ends: stdin stays open and unwritten for the whole download,
//! and ends when the stream ends, is cancelled or dropped, or the app goes away (a download
//! therefore needs `sh` beside `tar`; kind verifies no `tar` is left behind).
//!
//! An upload runs `tar -xmf - -C <dir>` with stdin on, sends the archive and closes stdin (the
//! remote end of input), then reads the exit status; a cancelled upload's `tar` sees its input
//! end and exits by itself.
//!
//! A container without `tar` (exit 126/127, or the runtime's `executable file not found`) is
//! [`Unsupported`](oxikube_domain::ErrorKind::Unsupported) with a message naming the container;
//! for one file, [`cat_download`](ExecTransferExt::cat_download) is the fallback (there is none
//! for upload).
//!
//! The helpers are written once over the [`ExecStreamPort`](super::ExecStreamPort) contract, so
//! the kube adapter (`oxikube_kube::remote::exec::KubeExec`) and the testkit fake run the same
//! code. They spawn
//! nothing: the adapter's websocket task is the pump, and its bounded pipes (32 KiB) are the
//! backpressure. A consumer that stops polling stops the remote program; memory stays at those
//! pipes plus the chunk in hand, whatever the file size. Callers poll from
//! `oxikube_runtime::spawn_kube` (abort on drop), never on the UI thread.
//!
//! # Safety
//!
//! * **Bytes only.** A download yields the raw archive. Unpacking it on the user's disk is the
//!   consumer's job and must refuse absolute paths, `..` and links that leave the destination
//!   (`kubectl cp` has had path-traversal CVEs): use `oxikube_app::transfer::safe_unpack`, never a
//!   plain unpack.
//! * **Upload mutates the pod.** [`tar_upload`](ExecTransferExt::tar_upload) writes into the
//!   container's filesystem. Call it only from `oxikube_app::transfer::TransferService::upload`,
//!   which takes the `Mutation` only `MutationGuard` hands out (non-negotiable 3); the app does
//!   not hand the stream port to views.
//! * **Nothing logged.** Neither the archive nor what the remote printed is logged; stderr only
//!   shapes the error message (file names, never content).

mod cancel;
mod download;
mod ext;
mod failure;
mod progress;
mod target;
#[cfg(test)]
mod tests;
mod upload;

use bytes::Bytes;
use futures::stream::BoxStream;
use oxikube_domain::OxiResult;

pub use cancel::{CancellationToken, CancelledFuture};
pub use ext::ExecTransferExt;
pub use progress::{PROGRESS_INTERVAL, ProgressFn, TransferProgress};
pub use target::{RemotePath, TransferOptions};

/// The bytes of a transfer, in order. An `Err` item ends it: the transfer failed (or was
/// cancelled) and what came before is incomplete.
pub type ByteStream = BoxStream<'static, OxiResult<Bytes>>;
