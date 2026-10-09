//! [`Array`] and [`Object`]: the container views of a [`JsonRef`](super::JsonRef).

use super::format::read_varint;
use super::keys;
use super::reader::JsonRef;

/// An array in a document: its length and an iterator over its elements.
#[derive(Clone, Copy)]
pub struct Array<'a> {
    /// The elements, back to back.
    items: &'a [u8],
    len: usize,
}

impl Array<'static> {
    /// An array with no elements: what an absent list reads as.
    pub const EMPTY: Array<'static> = Array { items: &[], len: 0 };
}

impl<'a> Array<'a> {
    /// From a container body (it starts with the element count).
    pub(super) fn open(body: &'a [u8]) -> Option<Self> {
        let (count, used) = read_varint(body)?;
        Some(Self {
            items: body.get(used..)?,
            len: usize::try_from(count).ok()?,
        })
    }

    /// Number of elements.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether there are no elements.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The elements in order.
    pub fn iter(&self) -> ArrayIter<'a> {
        ArrayIter {
            rest: self.items,
            left: self.len,
        }
    }

    /// The elements last to first. Elements have no back links, so each step walks from the
    /// start: meant for the short lists of a pod (containers, conditions), not for long arrays.
    pub fn iter_rev(&self) -> impl Iterator<Item = JsonRef<'a>> + use<'a> {
        let array = *self;
        (0..array.len).rev().filter_map(move |i| array.get(i))
    }

    /// The element at `index`.
    pub fn get(&self, index: usize) -> Option<JsonRef<'a>> {
        self.iter().nth(index)
    }

    /// The first element.
    pub fn first(&self) -> Option<JsonRef<'a>> {
        self.iter().next()
    }

    /// The last element.
    pub fn last(&self) -> Option<JsonRef<'a>> {
        self.iter().last()
    }
}

impl<'a> IntoIterator for Array<'a> {
    type Item = JsonRef<'a>;
    type IntoIter = ArrayIter<'a>;

    fn into_iter(self) -> ArrayIter<'a> {
        self.iter()
    }
}

/// Iterator over the elements of an [`Array`].
#[derive(Clone)]
pub struct ArrayIter<'a> {
    rest: &'a [u8],
    left: usize,
}

impl<'a> Iterator for ArrayIter<'a> {
    type Item = JsonRef<'a>;

    fn next(&mut self) -> Option<JsonRef<'a>> {
        if self.left == 0 || self.rest.is_empty() {
            self.left = 0;
            return None;
        }
        self.left -= 1;
        let value = JsonRef::at(self.rest);
        self.rest = self.rest.get(value.encoded_len()..).unwrap_or(&[]);
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }
}

impl ExactSizeIterator for ArrayIter<'_> {}

/// An object in a document: its length, lookup by key, and an iterator over its members in the
/// order they were in the source JSON.
#[derive(Clone, Copy)]
pub struct Object<'a> {
    /// The members (key then value), back to back.
    members: &'a [u8],
    len: usize,
}

/// A key as stored: a dictionary key or inline bytes.
struct RawKey<'a>(KeyRepr<'a>);

enum KeyRepr<'a> {
    Known(&'static str),
    Inline(&'a [u8]),
}

impl RawKey<'_> {
    fn matches(&self, key: &str) -> bool {
        match self.0 {
            KeyRepr::Known(known) => known == key,
            KeyRepr::Inline(bytes) => bytes == key.as_bytes(),
        }
    }
}

impl<'a> Object<'a> {
    /// From a container body (it starts with the member count).
    pub(super) fn open(body: &'a [u8]) -> Option<Self> {
        let (count, used) = read_varint(body)?;
        Some(Self {
            members: body.get(used..)?,
            len: usize::try_from(count).ok()?,
        })
    }

    /// Number of members.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether there are no members.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The members as `(key, value)` in source order.
    pub fn iter(&self) -> ObjectIter<'a> {
        ObjectIter {
            rest: self.members,
            left: self.len,
        }
    }

    /// The keys in source order.
    pub fn keys(&self) -> impl Iterator<Item = &'a str> + use<'a> {
        self.iter().map(|(key, _)| key)
    }

    /// The member `key`. Skips the members before it without decoding their values.
    pub fn get(&self, key: &str) -> Option<JsonRef<'a>> {
        let mut rest = self.members;
        for _ in 0..self.len {
            let (raw, after_key) = read_key(rest)?;
            let value = JsonRef::at(after_key);
            if raw.matches(key) {
                return Some(value);
            }
            rest = after_key.get(value.encoded_len()..)?;
        }
        None
    }

    /// Whether `key` is a member.
    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }
}

impl<'a> IntoIterator for Object<'a> {
    type Item = (&'a str, JsonRef<'a>);
    type IntoIter = ObjectIter<'a>;

    fn into_iter(self) -> ObjectIter<'a> {
        self.iter()
    }
}

/// Iterator over the members of an [`Object`].
#[derive(Clone)]
pub struct ObjectIter<'a> {
    rest: &'a [u8],
    left: usize,
}

impl<'a> Iterator for ObjectIter<'a> {
    type Item = (&'a str, JsonRef<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.left == 0 {
            return None;
        }
        self.left -= 1;
        let Some((raw, after_key)) = read_key(self.rest) else {
            self.left = 0;
            return None;
        };
        let key = match raw.0 {
            KeyRepr::Known(known) => known,
            KeyRepr::Inline(bytes) => std::str::from_utf8(bytes).unwrap_or_default(),
        };
        let value = JsonRef::at(after_key);
        self.rest = after_key.get(value.encoded_len()..).unwrap_or(&[]);
        Some((key, value))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }
}

impl ExactSizeIterator for ObjectIter<'_> {}

/// The key at the start of `buf` and the bytes after it.
fn read_key(buf: &[u8]) -> Option<(RawKey<'_>, &[u8])> {
    let (code, used) = read_varint(buf)?;
    let rest = buf.get(used..)?;
    if code == keys::INLINE {
        let (len, used) = read_varint(rest)?;
        let end = used.checked_add(usize::try_from(len).ok()?)?;
        let bytes = rest.get(used..end)?;
        return Some((RawKey(KeyRepr::Inline(bytes)), rest.get(end..)?));
    }
    Some((RawKey(KeyRepr::Known(keys::key_of(code)?)), rest))
}
