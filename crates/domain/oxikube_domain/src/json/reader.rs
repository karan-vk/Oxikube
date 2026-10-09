//! [`JsonRef`]: a borrowed read-only view of one value inside a [`JsonDoc`](super::JsonDoc).
//!
//! It mirrors the read half of [`serde_json::Value`] (`get`, `pointer`, `as_str`, `as_array`, ...)
//! and never allocates: strings are borrowed from the document, a lookup walks the encoded bytes
//! in place and skips whole subtrees by their stored length. It is `Copy` (two words), so it is
//! passed by value where a `&Value` used to be.
//!
//! A malformed buffer cannot happen (only the encoder writes them), but nothing here panics or
//! reads out of bounds if one did: every read is bounds-checked and a bad read looks like `null`.

use std::fmt;

use serde_json::{Map, Number, Value};

use super::format::{
    ARR, FALSE, FLOAT, INT, NULL, OBJ, SHORT_STR, STR, TRUE, UINT, read_varint, unzigzag,
};
use super::seq::{Array, Object};

/// What kind of value a [`JsonRef`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonKind {
    /// `null`, or an absent value.
    Null,
    /// `true` or `false`.
    Bool,
    /// An integer or a float.
    Number,
    /// A string.
    String,
    /// An array.
    Array,
    /// An object.
    Object,
}

/// A borrowed view of a value in a [`JsonDoc`](super::JsonDoc).
#[derive(Clone, Copy)]
pub struct JsonRef<'a> {
    /// Starts at the value's tag and may run on past its end (the rest of the document).
    bytes: &'a [u8],
}

impl JsonRef<'static> {
    /// `null`: what an absent subtree reads as.
    pub const NULL: JsonRef<'static> = JsonRef { bytes: &[NULL] };
}

impl<'a> JsonRef<'a> {
    pub(super) fn at(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    fn tag(self) -> u8 {
        self.bytes.first().copied().unwrap_or(NULL)
    }

    /// The bytes after the tag.
    fn payload(self) -> &'a [u8] {
        self.bytes.get(1..).unwrap_or(&[])
    }

    /// The kind of this value.
    pub fn kind(self) -> JsonKind {
        match self.tag() {
            FALSE | TRUE => JsonKind::Bool,
            INT | UINT | FLOAT => JsonKind::Number,
            STR => JsonKind::String,
            tag if tag >= SHORT_STR => JsonKind::String,
            ARR => JsonKind::Array,
            OBJ => JsonKind::Object,
            _ => JsonKind::Null,
        }
    }

    /// Whether this is `null` (or absent).
    pub fn is_null(self) -> bool {
        self.kind() == JsonKind::Null
    }

    /// Whether this is a string.
    pub fn is_string(self) -> bool {
        self.kind() == JsonKind::String
    }

    /// Whether this is an array.
    pub fn is_array(self) -> bool {
        self.kind() == JsonKind::Array
    }

    /// Whether this is an object.
    pub fn is_object(self) -> bool {
        self.kind() == JsonKind::Object
    }

    /// Whether this is a number.
    pub fn is_number(self) -> bool {
        self.kind() == JsonKind::Number
    }

    /// The boolean, if this is one.
    pub fn as_bool(self) -> Option<bool> {
        match self.tag() {
            FALSE => Some(false),
            TRUE => Some(true),
            _ => None,
        }
    }

    /// The integer, if this is an integer that fits `i64`.
    pub fn as_i64(self) -> Option<i64> {
        match self.tag() {
            INT => read_varint(self.payload()).map(|(n, _)| unzigzag(n)),
            // Only values above `i64::MAX` are stored as `UINT`.
            _ => None,
        }
    }

    /// The integer, if this is a non-negative integer.
    pub fn as_u64(self) -> Option<u64> {
        match self.tag() {
            UINT => read_varint(self.payload()).map(|(n, _)| n),
            INT => self.as_i64().and_then(|n| u64::try_from(n).ok()),
            _ => None,
        }
    }

    /// The number as a float: any number converts, as [`Value::as_f64`] does.
    pub fn as_f64(self) -> Option<f64> {
        match self.tag() {
            FLOAT => {
                let raw: [u8; 8] = self.payload().get(..8)?.try_into().ok()?;
                Some(f64::from_le_bytes(raw))
            }
            INT => self.as_i64().map(|n| n as f64),
            UINT => self.as_u64().map(|n| n as f64),
            _ => None,
        }
    }

    /// The string, borrowed from the document.
    pub fn as_str(self) -> Option<&'a str> {
        std::str::from_utf8(self.str_bytes()?).ok()
    }

    /// The bytes of the string, without checking that they are UTF-8 (they are: the encoder copied
    /// them from a `str`).
    pub(super) fn str_bytes(self) -> Option<&'a [u8]> {
        let tag = self.tag();
        if tag >= SHORT_STR {
            return self.payload().get(..usize::from(tag - SHORT_STR));
        }
        if tag != STR {
            return None;
        }
        let payload = self.payload();
        let (len, used) = read_varint(payload)?;
        payload.get(used..used.checked_add(usize::try_from(len).ok()?)?)
    }

    /// The array, if this is one.
    pub fn as_array(self) -> Option<Array<'a>> {
        (self.tag() == ARR)
            .then(|| Array::open(self.body()?))
            .flatten()
    }

    /// The object, if this is one.
    pub fn as_object(self) -> Option<Object<'a>> {
        (self.tag() == OBJ)
            .then(|| Object::open(self.body()?))
            .flatten()
    }

    /// The body of a container: the bytes after its length, exactly as long as the length says.
    fn body(self) -> Option<&'a [u8]> {
        let payload = self.payload();
        let (len, used) = read_varint(payload)?;
        payload.get(used..used.checked_add(usize::try_from(len).ok()?)?)
    }

    /// How many bytes this value takes in the document: how far to skip to the next sibling.
    pub(super) fn encoded_len(self) -> usize {
        let payload = self.payload();
        let after_tag = match self.tag() {
            NULL | FALSE | TRUE => 0,
            INT | UINT => read_varint(payload).map_or(payload.len(), |(_, used)| used),
            FLOAT => 8,
            STR => read_varint(payload).map_or(payload.len(), |(len, used)| {
                used.saturating_add(usize::try_from(len).unwrap_or(usize::MAX))
            }),
            ARR | OBJ => read_varint(payload).map_or(payload.len(), |(len, used)| {
                used.saturating_add(usize::try_from(len).unwrap_or(usize::MAX))
            }),
            tag if tag >= SHORT_STR => usize::from(tag - SHORT_STR),
            _ => 0,
        };
        1usize
            .saturating_add(after_tag)
            .min(self.bytes.len().max(1))
    }

    /// The member `key` of an object; `None` for a missing key or a value that is not an object.
    pub fn get(self, key: &str) -> Option<JsonRef<'a>> {
        self.as_object()?.get(key)
    }

    /// The element `index` of an array; `None` out of range or when this is not an array.
    pub fn index(self, index: usize) -> Option<JsonRef<'a>> {
        self.as_array()?.get(index)
    }

    /// The value at a JSON pointer ([RFC 6901]), for example `/spec/containers/0/name`; the empty
    /// pointer is this value. Same rules as [`Value::pointer`].
    ///
    /// [RFC 6901]: https://datatracker.ietf.org/doc/html/rfc6901
    pub fn pointer(self, pointer: &str) -> Option<JsonRef<'a>> {
        if pointer.is_empty() {
            return Some(self);
        }
        let rest = pointer.strip_prefix('/')?;
        let mut at = self;
        for token in rest.split('/') {
            at = if token.contains('~') {
                let token = token.replace("~1", "/").replace("~0", "~");
                at.step(&token)?
            } else {
                at.step(token)?
            };
        }
        Some(at)
    }

    fn step(self, token: &str) -> Option<JsonRef<'a>> {
        match self.kind() {
            JsonKind::Object => self.get(token),
            JsonKind::Array => {
                if token.starts_with('+') || (token.len() > 1 && token.starts_with('0')) {
                    return None;
                }
                self.index(token.parse().ok()?)
            }
            _ => None,
        }
    }

    /// This value as an owned [`Value`] (allocates the whole subtree; for cold paths).
    pub fn to_value(self) -> Value {
        match self.kind() {
            JsonKind::Null => Value::Null,
            JsonKind::Bool => Value::Bool(self.as_bool().unwrap_or(false)),
            JsonKind::Number => self.number().map_or(Value::Null, Value::Number),
            JsonKind::String => Value::String(self.as_str().unwrap_or_default().to_owned()),
            JsonKind::Array => Value::Array(
                self.as_array()
                    .map(|a| a.iter().map(JsonRef::to_value).collect())
                    .unwrap_or_default(),
            ),
            JsonKind::Object => {
                let Some(object) = self.as_object() else {
                    return Value::Null;
                };
                let mut map = Map::with_capacity(object.len());
                for (key, value) in object.iter() {
                    map.insert(key.to_owned(), value.to_value());
                }
                Value::Object(map)
            }
        }
    }

    /// Whether `self` and `other` are the same JSON value, as `Value`'s `==` says: objects are
    /// equal whatever the order of their keys, an integer is not a float, and so on.
    pub fn same_value(self, other: JsonRef<'_>) -> bool {
        match (self.kind(), other.kind()) {
            (JsonKind::Null, JsonKind::Null) => true,
            (JsonKind::Bool, JsonKind::Bool) => self.as_bool() == other.as_bool(),
            (JsonKind::Number, JsonKind::Number) => {
                // Same tag, same payload: an integer never equals a float (as in `Number`).
                self.tag() == other.tag() && self.number() == other.number()
            }
            (JsonKind::String, JsonKind::String) => self.str_bytes() == other.str_bytes(),
            (JsonKind::Array, JsonKind::Array) => match (self.as_array(), other.as_array()) {
                (Some(a), Some(b)) => {
                    a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.same_value(y))
                }
                _ => false,
            },
            (JsonKind::Object, JsonKind::Object) => match (self.as_object(), other.as_object()) {
                (Some(a), Some(b)) => {
                    a.len() == b.len()
                        && a.iter()
                            .all(|(key, x)| b.get(key).is_some_and(|y| x.same_value(y)))
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn number(self) -> Option<Number> {
        match self.tag() {
            INT => self.as_i64().map(Number::from),
            UINT => self.as_u64().map(Number::from),
            _ => self.as_f64().and_then(Number::from_f64),
        }
    }
}

impl fmt::Debug for JsonRef<'_> {
    /// Names the kind only, never the content, so a stray `{:?}` cannot leak Secret data.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JsonRef({:?})", self.kind())
    }
}

impl PartialEq<str> for JsonRef<'_> {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == Some(other)
    }
}

impl PartialEq<&str> for JsonRef<'_> {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == Some(*other)
    }
}
