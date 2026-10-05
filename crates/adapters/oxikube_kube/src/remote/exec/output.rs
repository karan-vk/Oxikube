//! Remote stdout and stderr as streams of byte chunks.

use futures::StreamExt;
use futures::stream;
use oxikube_domain::OxiError;
use oxikube_ports::exec::OutputStream;
use tokio::io::{AsyncRead, AsyncReadExt};

use super::params::STREAM_BUFFER;

/// Every chunk the stream yields is one read from the pipe, copied into a vector of exactly
/// its length: no allocation per byte, none per empty poll, and a chunk is as large as what
/// was available (up to [`STREAM_BUFFER`]) so a burst is not cut into small pieces. The stream
/// ends when the pipe closes (the process ended or the session was dropped) and after the
/// first read error.
pub(super) fn chunks<R>(reader: R) -> OutputStream
where
    R: AsyncRead + Send + Unpin + 'static,
{
    let state = Some((reader, vec![0u8; STREAM_BUFFER].into_boxed_slice()));
    stream::unfold(state, |state| async move {
        let (mut reader, mut buffer) = state?;
        match reader.read(&mut buffer).await {
            Ok(0) => None,
            Ok(n) => Some((Ok(buffer[..n].to_vec()), Some((reader, buffer)))),
            Err(err) => Some((
                Err(OxiError::network(format!(
                    "reading the remote output failed: {err}"
                ))),
                None,
            )),
        }
    })
    // Polling past the end is harmless, not a panic.
    .fuse()
    .boxed()
}
