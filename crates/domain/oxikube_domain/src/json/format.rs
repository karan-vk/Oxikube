//! The byte format of a [`JsonDoc`](super::JsonDoc): tags and LEB128 varints.
//!
//! ```text
//! value   := null | false | true                    1 byte tag
//!          | INT zigzag-varint | UINT varint        tag, then the number
//!          | FLOAT f64-le                           tag, 8 bytes
//!          | STR varint(len) bytes                  tag, length, UTF-8 (any length)
//!          | 0x80|len bytes                         a string of up to 127 bytes, length in the tag
//!          | ARR varint(body) varint(n) value*n     body = bytes after its own varint
//!          | OBJ varint(body) varint(n) (key value)*n
//! key     := varint(0) varint(len) bytes            inline key
//!          | varint(code)                           dictionary key `KEYS[code - 1]`
//! ```
//!
//! Containers carry the byte length of their body, so skipping a whole subtree is one varint read
//! and a lookup of one key in an object never decodes the values it passes over.

/// `null`.
pub(super) const NULL: u8 = 0;
/// `false`.
pub(super) const FALSE: u8 = 1;
/// `true`.
pub(super) const TRUE: u8 = 2;
/// A signed integer, zigzag varint.
pub(super) const INT: u8 = 3;
/// An unsigned integer above `i64::MAX`, varint.
pub(super) const UINT: u8 = 4;
/// A float, 8 bytes little endian.
pub(super) const FLOAT: u8 = 5;
/// A string with a varint length.
pub(super) const STR: u8 = 6;
/// An array.
pub(super) const ARR: u8 = 7;
/// An object.
pub(super) const OBJ: u8 = 8;
/// Tags from here are strings whose length is `tag - SHORT_STR`.
pub(super) const SHORT_STR: u8 = 0x80;
/// The longest string that uses the short form.
pub(super) const MAX_SHORT: usize = 127;
/// Bytes reserved for a container's body length while its body is written (35 bits).
pub(super) const LEN_RESERVE: usize = 5;

/// Appends `n` as a LEB128 varint.
pub(super) fn put_varint(out: &mut Vec<u8>, mut n: u64) {
    while n >= 0x80 {
        out.push((n & 0x7f) as u8 | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

/// `n` as a varint in a stack buffer: the bytes and how many are used.
pub(super) fn varint_bytes(mut n: u64) -> ([u8; 10], usize) {
    let mut buf = [0u8; 10];
    let mut used = 0;
    while n >= 0x80 {
        buf[used] = (n & 0x7f) as u8 | 0x80;
        used += 1;
        n >>= 7;
    }
    buf[used] = n as u8;
    (buf, used + 1)
}

/// The varint at the start of `buf` and the bytes it took; `None` when `buf` ends inside it.
pub(super) fn read_varint(buf: &[u8]) -> Option<(u64, usize)> {
    let mut n = 0u64;
    for (i, byte) in buf.iter().enumerate().take(10) {
        n |= u64::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            return Some((n, i + 1));
        }
    }
    None
}

/// Zigzag: small negative numbers stay small.
pub(super) fn zigzag(n: i64) -> u64 {
    ((n << 1) ^ (n >> 63)) as u64
}

/// Inverse of [`zigzag`].
pub(super) fn unzigzag(n: u64) -> i64 {
    ((n >> 1) as i64) ^ -((n & 1) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_round_trip() {
        for n in [
            0,
            1,
            127,
            128,
            300,
            16_383,
            16_384,
            u32::MAX as u64,
            u64::MAX,
        ] {
            let mut out = Vec::new();
            put_varint(&mut out, n);
            assert_eq!(read_varint(&out), Some((n, out.len())), "{n}");
        }
    }

    #[test]
    fn a_truncated_varint_is_none() {
        assert_eq!(read_varint(&[0x80]), None);
        assert_eq!(read_varint(&[]), None);
    }

    #[test]
    fn zigzag_round_trips() {
        for n in [0, 1, -1, 63, -64, i64::MAX, i64::MIN] {
            assert_eq!(unzigzag(zigzag(n)), n);
        }
    }
}
