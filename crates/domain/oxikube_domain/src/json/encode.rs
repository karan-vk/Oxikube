//! Writes a [`serde_json::Value`] in the compact [format](super::format).

use serde_json::{Map, Value};

use super::format::{
    ARR, FALSE, FLOAT, INT, LEN_RESERVE, MAX_SHORT, NULL, OBJ, SHORT_STR, STR, TRUE, UINT,
    put_varint, varint_bytes, zigzag,
};
use super::keys::{self, INLINE};

/// The encoding of `value`.
pub(super) fn encode(value: &Value) -> Vec<u8> {
    let mut out = Vec::with_capacity(256);
    put(&mut out, value);
    out
}

fn put(out: &mut Vec<u8>, value: &Value) {
    match value {
        Value::Null => out.push(NULL),
        Value::Bool(false) => out.push(FALSE),
        Value::Bool(true) => out.push(TRUE),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.push(INT);
                put_varint(out, zigzag(i));
            } else if let Some(u) = n.as_u64() {
                out.push(UINT);
                put_varint(out, u);
            } else {
                out.push(FLOAT);
                out.extend_from_slice(&n.as_f64().unwrap_or(0.0).to_le_bytes());
            }
        }
        Value::String(s) => put_str(out, s),
        Value::Array(items) => container(out, ARR, items.len(), |out| {
            for item in items {
                put(out, item);
            }
        }),
        Value::Object(map) => put_object(out, map),
    }
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    if s.len() <= MAX_SHORT {
        out.push(SHORT_STR | s.len() as u8);
    } else {
        out.push(STR);
        put_varint(out, s.len() as u64);
    }
    out.extend_from_slice(s.as_bytes());
}

fn put_object(out: &mut Vec<u8>, map: &Map<String, Value>) {
    container(out, OBJ, map.len(), |out| {
        for (key, value) in map {
            match keys::code_of(key) {
                Some(code) => put_varint(out, code),
                None => {
                    put_varint(out, INLINE);
                    put_varint(out, key.len() as u64);
                    out.extend_from_slice(key.as_bytes());
                }
            }
            put(out, value);
        }
    });
}

/// Writes `tag`, the body length, `count` and then whatever `body` appends. The length is not
/// known until the body is written, so room for the widest length is left and closed up after.
fn container(out: &mut Vec<u8>, tag: u8, count: usize, body: impl FnOnce(&mut Vec<u8>)) {
    out.push(tag);
    let len_at = out.len();
    out.extend_from_slice(&[0; LEN_RESERVE]);
    let body_at = out.len();
    put_varint(out, count as u64);
    body(out);
    let (len, used) = varint_bytes((out.len() - body_at) as u64);
    out[len_at..len_at + used].copy_from_slice(&len[..used]);
    if used < LEN_RESERVE {
        out.copy_within(body_at.., len_at + used);
        out.truncate(out.len() - (LEN_RESERVE - used));
    }
}
