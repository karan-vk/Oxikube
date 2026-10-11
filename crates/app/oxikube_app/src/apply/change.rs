//! What a dry run says an apply does to an object: [`Change`], from the live object and the
//! object as the server would store it.

use serde_json::Value;

/// The fields the server owns and a manifest never sets, as paths from the object's root. A
/// dry run's result differs from the live object in these alone when the apply changes nothing, so
/// the apply preview, the diff vs live (`oxikube_editor::diff`) and [`Change`] all ignore them.
pub const SERVER_OWNED_FIELDS: &[&[&str]] = &[
    &["metadata", "managedFields"],
    &["metadata", "resourceVersion"],
    &["metadata", "generation"],
    &["metadata", "uid"],
    &["metadata", "creationTimestamp"],
    &["metadata", "selfLink"],
    &["status"],
];

/// What an apply does to the object, from the server's dry run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The object does not exist: the apply creates it.
    Create,
    /// The object exists and the apply changes it.
    Update,
    /// The object exists and the apply leaves it as it is.
    Unchanged,
}

impl Change {
    /// The change of storing `result` over `live` (`None` when there is no live object). Secret
    /// values in both are fingerprints already (`mask`), so equal values compare equal.
    pub fn between(live: Option<&Value>, result: &Value) -> Self {
        match live {
            None => Change::Create,
            Some(live) if without_server_fields(live) == without_server_fields(result) => {
                Change::Unchanged
            }
            Some(_) => Change::Update,
        }
    }
}

/// `value` without [`SERVER_OWNED_FIELDS`].
fn without_server_fields(value: &Value) -> Value {
    let mut out = value.clone();
    for path in SERVER_OWNED_FIELDS {
        let Some((last, parents)) = path.split_last() else {
            continue;
        };
        let mut at = Some(&mut out);
        for key in parents {
            at = at.and_then(|node| node.get_mut(*key));
        }
        if let Some(map) = at.and_then(Value::as_object_mut) {
            map.shift_remove(*last);
        }
    }
    out
}
