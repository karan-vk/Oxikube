//! [`tar_file_stream`]: one local file as a tar stream, for an upload, built as it is sent.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;
use futures::{StreamExt as _, stream};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::exec::ByteStream;
use tar::{Builder, EntryType, Header};

/// Two zero blocks: the end of a tar archive.
const END_OF_ARCHIVE: usize = 1024;

/// The header block(s) of a regular file `name` of `size` bytes and `mode`: a GNU header, after
/// a long-name record when the name needs one.
fn header_bytes(name: &str, size: u64, mode: u32) -> OxiResult<Vec<u8>> {
    let mut header = Header::new_gnu();
    header.set_entry_type(EntryType::Regular);
    header.set_size(size);
    header.set_mode(mode & 0o7777);
    let mut builder = Builder::new(Vec::new());
    // The data is streamed later: append the header with no bytes after it.
    builder
        .append_data(&mut header, name, std::io::empty())
        .map_err(|error| OxiError::validation(format!("{name} cannot be archived: {error}")))?;
    let mut bytes = builder
        .into_inner()
        .map_err(|error| OxiError::internal(format!("building a tar header failed: {error}")))?;
    // `into_inner` ends the archive (two zero blocks); this archive goes on with the data.
    let end = bytes.len().saturating_sub(END_OF_ARCHIVE);
    if bytes[end..].iter().any(|byte| *byte != 0) {
        return Err(OxiError::internal("a tar header did not end as expected"));
    }
    bytes.truncate(end);
    Ok(bytes)
}

/// A tar stream holding one regular file `name` (a plain name, no `/`) of exactly `size` bytes
/// taken from `content`, with permission bits `mode`. It is built as it is read, so a large file
/// is never held in memory: the header, `content`'s chunks unchanged, then the padding and the
/// end of the archive. Feed it to `TransferService::upload` (or
/// [`ExecTransferExt::tar_upload`](oxikube_ports::ExecTransferExt::tar_upload)).
///
/// # Errors
///
/// `Validation` for a name that is empty, `.`/`..`, or holds `/` or NUL. The stream yields an
/// error and ends when `content` fails or does not hold exactly `size` bytes (the remote `tar`
/// then refuses the archive).
pub fn tar_file_stream(
    name: &str,
    mode: u32,
    size: u64,
    content: ByteStream,
) -> OxiResult<ByteStream> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\0']) {
        return Err(OxiError::validation(format!(
            "{name:?} is not a file name to upload"
        )));
    }
    let header = Bytes::from(header_bytes(name, size, mode)?);
    let seen = Arc::new(AtomicU64::new(0));
    let counted = seen.clone();
    let body = content.map(move |chunk| {
        let chunk = chunk?;
        let total = counted.fetch_add(chunk.len() as u64, Ordering::Relaxed) + chunk.len() as u64;
        if total > size {
            return Err(OxiError::conflict(
                "the file grew while it was being uploaded",
            ));
        }
        Ok(chunk)
    });
    let end = stream::once(async move {
        let sent = seen.load(Ordering::Relaxed);
        if sent != size {
            return Err(OxiError::conflict(format!(
                "the file changed while it was being uploaded ({sent} of {size} bytes)"
            )));
        }
        let padding = usize::try_from((512 - size % 512) % 512).unwrap_or(0);
        Ok(Bytes::from(vec![0u8; padding + END_OF_ARCHIVE]))
    });
    Ok(stream::once(async move { Ok(header) })
        .chain(body)
        .chain(end)
        .boxed())
}
