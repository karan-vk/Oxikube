//! Transfer tests: the unpack and the packer over the in-memory `FakeFsPort`, the service over
//! a real session manager and the `FakeExecStreamPort`.

mod pack;
mod service;
mod unpack;

use bytes::Bytes;
use futures::{StreamExt as _, stream};
use oxikube_ports::exec::ByteStream;

/// `bytes` as a stream of chunks of `size` (to cross every block boundary).
pub(super) fn chunked(bytes: &[u8], size: usize) -> ByteStream {
    let chunks: Vec<_> = bytes
        .chunks(size)
        .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
        .collect();
    stream::iter(chunks).boxed()
}

/// Collects a stream, failing the test on an error.
pub(super) async fn collect(stream: ByteStream) -> Vec<u8> {
    stream
        .map(|chunk| chunk.expect("a chunk").to_vec())
        .concat()
        .await
}

/// A tar archive built with the tar crate: `(path, contents)` regular files.
pub(super) fn archive_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, contents) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        builder
            .append_data(&mut header, path, *contents)
            .expect("append");
    }
    builder.into_inner().expect("finish")
}
