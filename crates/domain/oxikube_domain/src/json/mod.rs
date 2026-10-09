//! [`JsonDoc`]: a Kubernetes object's JSON held compactly, and read in place through [`JsonRef`].
//!
//! A watched object used to be a [`serde_json::Value`] tree: one heap node per field, a `String`
//! per key and per text value and a hash table per object, about 13 to 17 KB of small allocations
//! for a pod (E07-P603). A [`JsonDoc`] is one immutable byte buffer in a tagged, varint format
//! (see [`format`]) with the common Kubernetes keys replaced by small numbers ([`keys`]): the same
//! pod is a few hundred bytes to a couple of KB in one allocation.
//!
//! Reading does not decode: [`JsonRef`] walks the bytes, borrows strings from them and skips
//! subtrees by their stored length, so the table's cells, the health tally and the view-models read
//! a field without allocating. [`JsonDoc::to_value`] builds the full tree for the few cold paths
//! that need one (editing, schemas), and [`serde::Serialize`] streams a document straight to YAML
//! or JSON without building it.
//!
//! | File | Holds |
//! |---|---|
//! | `format` | tags and varints |
//! | `keys` | the key dictionary |
//! | `encode` | `Value` to bytes |
//! | `reader` | [`JsonRef`] |
//! | `seq` | [`Array`], [`Object`] and their iterators |
//! | `serde_impl` | `Serialize` for a view, `Serialize` / `Deserialize` for a document |

mod encode;
mod format;
mod keys;
mod reader;
mod seq;
mod serde_impl;
#[cfg(test)]
mod tests;

use std::fmt;
use std::sync::Arc;

use serde_json::Value;

pub use reader::{JsonKind, JsonRef};
pub use seq::{Array, ArrayIter, Object, ObjectIter};

/// An immutable JSON document in compact form. Cloning shares the bytes.
///
/// `==` is JSON equality, like [`Value`]'s: the bytes of equal documents match unless their keys are
/// in another order, which the member-by-member fallback accepts. `Debug` prints the size only, never the content, so a Secret cannot reach a log.
#[derive(Clone)]
pub struct JsonDoc {
    bytes: Arc<[u8]>,
}

impl JsonDoc {
    /// Encodes `value`.
    pub fn from_value(value: &Value) -> Self {
        Self {
            bytes: Arc::from(encode::encode(value)),
        }
    }

    /// The document's root value.
    pub fn root(&self) -> JsonRef<'_> {
        JsonRef::at(&self.bytes)
    }

    /// The whole document as a [`Value`] tree. Allocates the tree; for cold paths.
    pub fn to_value(&self) -> Value {
        self.root().to_value()
    }

    /// Size of the encoding in bytes.
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether `other` is a clone of this document (same buffer).
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.bytes, &other.bytes)
    }
}

impl PartialEq for JsonDoc {
    /// The same JSON value, as `Value`'s `==` says it (key order does not matter). Identical bytes
    /// settle it at once; only documents that differ in bytes are compared member by member.
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other) || self.bytes == other.bytes || self.root().same_value(other.root())
    }
}

impl Eq for JsonDoc {}

impl fmt::Debug for JsonDoc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JsonDoc({} bytes)", self.bytes.len())
    }
}

impl From<&Value> for JsonDoc {
    fn from(value: &Value) -> Self {
        Self::from_value(value)
    }
}

impl From<Value> for JsonDoc {
    fn from(value: Value) -> Self {
        Self::from_value(&value)
    }
}
