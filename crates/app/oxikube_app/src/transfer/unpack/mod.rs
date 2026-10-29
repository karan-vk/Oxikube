//! [`safe_unpack`]: a downloaded tar stream onto the user's disk, refusing anything that would
//! land outside the destination.
//!
//! `kubectl cp` has had path-traversal CVEs: a hostile (or compromised) container controls the
//! archive, so every entry is checked before a byte is written.
//!
//! The rules per entry are in [`safe_unpack`]'s docs.
//!
//! File modes and owners are not applied (the port writes plain files). What is unpacked is never
//! logged; the report holds paths and sizes only.

mod path;
mod reader;

use std::path::{Path, PathBuf};

use futures::channel::mpsc;
use futures::{SinkExt as _, StreamExt as _};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::FsPort;
use oxikube_ports::exec::ByteStream;

use path::{entry_path, link_stays_inside};
use reader::{Entry, Kind, TarReader};

/// The most skipped entries listed one by one; past it they are only counted.
const MAX_LISTED_SKIPS: usize = 1000;

/// An archive entry that was not created, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skipped {
    /// A symbolic link that would stay inside the destination. Links are never created.
    Link(String),
    /// A symbolic link to an absolute path or above the destination.
    UnsafeLink(String),
    /// A hard link.
    HardLink(String),
    /// A device, FIFO, sparse file or other special entry.
    Special(String),
}

/// What [`safe_unpack`] wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnpackReport {
    /// Files written, relative to the destination, in archive order.
    pub files: Vec<PathBuf>,
    /// File bytes written.
    pub bytes: u64,
    /// Entries not created (the first thousand).
    pub skipped: Vec<Skipped>,
    /// How many entries were skipped in all.
    pub skipped_count: usize,
}

impl UnpackReport {
    fn skip(&mut self, skipped: Skipped) {
        self.skipped_count += 1;
        if self.skipped.len() < MAX_LISTED_SKIPS {
            self.skipped.push(skipped);
        }
    }
}

/// Unpacks the tar `archive` under the directory `dest`, through `fs`, refusing every entry that
/// would land outside it. Streams: each file is written as its data arrives, never held whole.
///
/// | Entry | What happens |
/// |---|---|
/// | file with a plain relative path | written under the destination through [`FsPort::write_stream`] (atomic per file, streamed) |
/// | directory | nothing to do: directories are created with the files in them (an empty one is not created) |
/// | absolute path, `..`, a `\` or (on Windows) a drive letter | **the unpack stops** with a `Validation` error naming the entry; files written before it stay |
/// | symbolic link | never created: listed in [`UnpackReport::skipped`] as [`Skipped::Link`] when it would stay inside the destination, [`Skipped::UnsafeLink`] when it points outside (absolute, or climbing out) |
/// | hard link, device, FIFO, sparse file | never created: listed as [`Skipped::HardLink`] / [`Skipped::Special`] |
///
/// File modes and owners are not applied (the port writes plain files).
///
/// # Errors
///
/// `Validation` for an entry whose path escapes `dest` (the unpack stops there), for a stream
/// that is not a tar archive, ends mid-entry or goes on for more than 1 MiB past the archive's
/// end marker; the archive stream's own error (a failed or cancelled download) as it arrives,
/// including one after the end marker (the stream is read to its end, so a remote `tar` that
/// wrote a whole archive but exited non-zero, having skipped unreadable files, is an error, not a
/// success); `fs`'s errors writing a file. A file being written when an error arrives is
/// abandoned (the port's atomic write), files finished before stay.
pub async fn safe_unpack(
    fs: &dyn FsPort,
    dest: &Path,
    archive: ByteStream,
) -> OxiResult<UnpackReport> {
    let mut reader = TarReader::new(archive);
    let mut report = UnpackReport::default();
    while let Some(entry) = reader.next_entry().await? {
        let shown = String::from_utf8_lossy(&entry.path).into_owned();
        let relative = entry_path(&entry.path).map_err(|refusal| {
            OxiError::validation(format!(
                "the archive entry {shown} was refused because {}; nothing after it was unpacked",
                refusal.reason()
            ))
        })?;
        match entry.kind {
            Kind::File => {
                let Some(relative) = relative else {
                    return Err(OxiError::validation(format!(
                        "the archive entry {shown} is a file without a name"
                    )));
                };
                write_file(fs, &dest.join(&relative), &mut reader).await?;
                report.bytes += entry.size;
                report.files.push(relative);
            }
            Kind::Directory => {}
            Kind::Symlink => report.skip(link_skip(&entry, relative.as_deref(), shown)),
            Kind::HardLink => report.skip(Skipped::HardLink(shown)),
            Kind::Other => report.skip(Skipped::Special(shown)),
        }
    }
    // The download's outcome (the remote `tar`'s exit status) arrives after the archive's end
    // marker: read it, or a tar that skipped unreadable files would pass for a success.
    reader.finish().await?;
    Ok(report)
}

fn link_skip(entry: &Entry, relative: Option<&Path>, shown: String) -> Skipped {
    let inside = match (relative, &entry.link) {
        (Some(relative), Some(target)) => link_stays_inside(relative, target),
        _ => false,
    };
    if inside {
        Skipped::Link(shown)
    } else {
        Skipped::UnsafeLink(shown)
    }
}

/// Streams the current entry's data into `path` through a small bounded channel, so the write
/// and the read run together without buffering the file.
async fn write_file(fs: &dyn FsPort, path: &Path, reader: &mut TarReader) -> OxiResult<()> {
    let (mut tx, rx) = mpsc::channel::<OxiResult<Vec<u8>>>(2);
    let write = fs.write_stream(path, rx.boxed());
    let feed = async move {
        loop {
            match reader.data().await {
                Ok(Some(part)) => {
                    if tx.send(Ok(Vec::from(part))).await.is_err() {
                        // The write gave up (its own error is returned below).
                        return Ok(());
                    }
                }
                Ok(None) => return Ok(()),
                Err(error) => {
                    // Abandons the file; the error itself is returned below.
                    let abandon = OxiError::conflict("the archive failed mid-file");
                    let _ = tx.send(Err(abandon)).await;
                    return Err(error);
                }
            }
        }
    };
    let (written, fed) = futures::join!(write, feed);
    fed?;
    written
}
