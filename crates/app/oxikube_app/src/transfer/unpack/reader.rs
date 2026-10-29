//! A streaming tar reader: headers and data from a [`ByteStream`], one block at a time, without
//! holding more than the chunk in hand. The tar crate parses each 512-byte header; the framing,
//! GNU long names and pax records are read here.

use bytes::Bytes;
use futures::StreamExt as _;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::exec::ByteStream;
use tar::{EntryType, Header, PaxExtensions};

/// A tar block.
const BLOCK: usize = 512;

/// The largest metadata record (a GNU long name, a pax header) read into memory.
const MAX_META_BYTES: u64 = 64 * 1024;

/// What an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    File,
    Directory,
    Symlink,
    HardLink,
    /// Devices, FIFOs, sparse files and anything else this unpack does not create.
    Other,
}

/// One entry's header, with its long name, link and size already resolved.
#[derive(Debug)]
pub(super) struct Entry {
    pub(super) path: Vec<u8>,
    pub(super) link: Option<Vec<u8>>,
    pub(super) kind: Kind,
    pub(super) size: u64,
}

/// Reads a tar archive from a stream of chunks.
pub(super) struct TarReader {
    input: ByteStream,
    current: Bytes,
    /// Data bytes of the current entry not read yet, and the padding after them.
    remaining: u64,
    padding: u64,
}

fn truncated() -> OxiError {
    OxiError::validation("the archive ended in the middle of an entry")
}

fn not_tar() -> OxiError {
    OxiError::validation("the download is not a tar archive (a header checksum does not match)")
}

/// Whether `block`'s stored checksum matches its bytes (the checksum field counts as spaces).
fn checksum_matches(block: &[u8; BLOCK], header: &Header) -> bool {
    let Ok(stored) = header.cksum() else {
        return false;
    };
    let sum: u32 = block
        .iter()
        .enumerate()
        .map(|(at, byte)| {
            if (148..156).contains(&at) {
                32
            } else {
                u32::from(*byte)
            }
        })
        .sum();
    sum == stored
}

/// Strips the NUL terminator (and anything after it) of a GNU long name.
fn until_nul(mut bytes: Vec<u8>) -> Vec<u8> {
    if let Some(end) = bytes.iter().position(|byte| *byte == 0) {
        bytes.truncate(end);
    }
    bytes
}

impl TarReader {
    pub(super) fn new(input: ByteStream) -> Self {
        Self {
            input,
            current: Bytes::new(),
            remaining: 0,
            padding: 0,
        }
    }

    /// Makes `current` non-empty; `false` at the end of the input.
    async fn fill(&mut self) -> OxiResult<bool> {
        while self.current.is_empty() {
            match self.input.next().await {
                Some(chunk) => self.current = chunk?,
                None => return Ok(false),
            }
        }
        Ok(true)
    }

    /// Up to `max` bytes (at least one), as a slice of the chunk in hand: no copy.
    async fn take(&mut self, max: u64) -> OxiResult<Bytes> {
        if !self.fill().await? {
            return Err(truncated());
        }
        let n = usize::try_from(max)
            .unwrap_or(usize::MAX)
            .min(self.current.len());
        Ok(self.current.split_to(n))
    }

    async fn skip(&mut self, mut n: u64) -> OxiResult<()> {
        while n > 0 {
            n -= self.take(n).await?.len() as u64;
        }
        Ok(())
    }

    /// The next block, or `None` at a clean end of input.
    async fn block(&mut self) -> OxiResult<Option<[u8; BLOCK]>> {
        if !self.fill().await? {
            return Ok(None);
        }
        let mut block = [0u8; BLOCK];
        let mut filled = 0;
        while filled < BLOCK {
            let part = self.take((BLOCK - filled) as u64).await?;
            block[filled..filled + part.len()].copy_from_slice(&part);
            filled += part.len();
        }
        Ok(Some(block))
    }

    /// The data of a metadata record of `size` bytes, plus its padding skipped.
    async fn record(&mut self, size: u64) -> OxiResult<Vec<u8>> {
        if size > MAX_META_BYTES {
            return Err(OxiError::validation(
                "the archive has a metadata record too large to read",
            ));
        }
        let mut data = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
        while (data.len() as u64) < size {
            let part = self.take(size - data.len() as u64).await?;
            data.extend_from_slice(&part);
        }
        self.skip(padding_of(size)).await?;
        Ok(data)
    }

    /// The next entry's header, after skipping what is left of the previous one. `None` at the
    /// end of the archive (a zero block, or the end of the input).
    pub(super) async fn next_entry(&mut self) -> OxiResult<Option<Entry>> {
        self.skip(self.remaining + self.padding).await?;
        self.remaining = 0;
        self.padding = 0;
        let mut long_path = None;
        let mut long_link = None;
        let mut pax_size = None;
        loop {
            let Some(block) = self.block().await? else {
                return Ok(None);
            };
            if block.iter().all(|byte| *byte == 0) {
                return Ok(None);
            }
            let header = Header::from_byte_slice(&block);
            if !checksum_matches(&block, header) {
                return Err(not_tar());
            }
            let size = header.entry_size().map_err(|_| not_tar())?;
            match header.entry_type() {
                EntryType::GNULongName => long_path = Some(until_nul(self.record(size).await?)),
                EntryType::GNULongLink => long_link = Some(until_nul(self.record(size).await?)),
                EntryType::XHeader => {
                    let record = self.record(size).await?;
                    for extension in PaxExtensions::new(&record).flatten() {
                        match extension.key_bytes() {
                            b"path" => long_path = Some(extension.value_bytes().to_vec()),
                            b"linkpath" => long_link = Some(extension.value_bytes().to_vec()),
                            b"size" => {
                                pax_size = extension.value().ok().and_then(|v| v.parse().ok());
                            }
                            _ => {}
                        }
                    }
                }
                EntryType::XGlobalHeader => self.skip(size + padding_of(size)).await?,
                kind => {
                    let size = pax_size.unwrap_or(size);
                    self.remaining = size;
                    self.padding = padding_of(size);
                    return Ok(Some(Entry {
                        path: long_path.unwrap_or_else(|| header.path_bytes().into_owned()),
                        link: long_link
                            .or_else(|| header.link_name_bytes().map(|link| link.into_owned())),
                        kind: match kind {
                            EntryType::Regular | EntryType::Continuous => Kind::File,
                            EntryType::Directory => Kind::Directory,
                            EntryType::Symlink => Kind::Symlink,
                            EntryType::Link => Kind::HardLink,
                            _ => Kind::Other,
                        },
                        size,
                    }));
                }
            }
        }
    }

    /// The next piece of the current entry's data, `None` once it was all read.
    pub(super) async fn data(&mut self) -> OxiResult<Option<Bytes>> {
        if self.remaining == 0 {
            return Ok(None);
        }
        let part = self.take(self.remaining).await?;
        self.remaining -= part.len() as u64;
        Ok(Some(part))
    }
}

/// The zero bytes after `size` bytes of data, up to the next block.
pub(super) fn padding_of(size: u64) -> u64 {
    (BLOCK as u64 - size % BLOCK as u64) % BLOCK as u64
}
