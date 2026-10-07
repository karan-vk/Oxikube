//! Row content hashing for the row cache: what a viewport row shows, hashed with the
//! multiply-rotate hash rustc uses (fast on small keys, plenty for change detection).

use std::hash::{BuildHasherDefault, Hash, Hasher};

use crate::grid::TerminalSnapshot;

/// A `HashMap` hasher for keys that are already hashes.
pub(super) type FxBuild = BuildHasherDefault<FxHasher>;

/// A hash of what viewport `row` shows: its cells and combining marks.
pub(super) fn row_hash(snapshot: &TerminalSnapshot, row: usize) -> u64 {
    let mut hasher = FxHasher::default();
    let start = row * snapshot.columns;
    if let Some(cells) = snapshot.cells.get(start..start + snapshot.columns) {
        cells.hash(&mut hasher);
    }
    let end = start + snapshot.columns;
    let first = snapshot.zerowidth.partition_point(|&(i, _)| i < start);
    for &(index, mark) in snapshot.zerowidth[first..]
        .iter()
        .take_while(|(i, _)| *i < end)
    {
        (index - start, mark).hash(&mut hasher);
    }
    hasher.finish()
}

/// The multiply-rotate hash rustc uses: fast on small keys, plenty for change detection.
#[derive(Default)]
pub(super) struct FxHasher {
    hash: u64,
}

impl Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.write_u64(u64::from_le_bytes(word));
        }
    }

    fn write_u64(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }

    fn write_u8(&mut self, byte: u8) {
        self.write_u64(u64::from(byte));
    }

    fn write_u16(&mut self, word: u16) {
        self.write_u64(u64::from(word));
    }

    fn write_u32(&mut self, word: u32) {
        self.write_u64(u64::from(word));
    }

    fn write_usize(&mut self, word: usize) {
        self.write_u64(word as u64);
    }

    fn finish(&self) -> u64 {
        self.hash
    }
}
