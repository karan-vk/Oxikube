//! Editing a Secret decoded: the object with plaintext under `data`, and back.
//!
//! "Edit decoded" opens the editor on [`to_decoded`] of the object: each text value of `data` is
//! its plaintext and each binary one a read-only placeholder (`(binary, 12 bytes)`). The buffer
//! is the manifest, as everywhere in the editor; when it is applied, [`from_decoded`] writes every
//! `data` value back as base64 (a placeholder still in place takes the original value of its
//! key). Both are pure functions over the object's JSON, so they run on the background executor
//! and in plain unit tests. Nothing here writes or logs a value.

use serde_json::Value;

use super::decoded::{DecodedValue, MASKED_VALUE, SecretError, is_placeholder, is_secret};

/// The annotation `kubectl apply` writes: the whole applied manifest, a Secret's data included.
pub const LAST_APPLIED: &str = "kubectl.kubernetes.io/last-applied-configuration";

/// Replaces every value under `data` of the Secret `object` with its plaintext (a binary value
/// with its placeholder) and drops the `last-applied-configuration` annotation, which carries a
/// base64 copy of the old values. Returns whether the annotation was removed. Any other object
/// is left alone.
pub fn to_decoded(object: &mut Value) -> bool {
    if !is_secret(object) {
        return false;
    }
    if let Some(data) = object.get_mut("data").and_then(Value::as_object_mut) {
        for value in data.values_mut() {
            let Some(encoded) = value.as_str() else {
                continue;
            };
            *value = Value::String(DecodedValue::from_base64(encoded).shown());
        }
    }
    object
        .pointer_mut("/metadata/annotations")
        .and_then(Value::as_object_mut)
        .is_some_and(|annotations| annotations.shift_remove(LAST_APPLIED).is_some())
}

/// Writes every value under `data` of the Secret `object` as base64. A value that is still a
/// placeholder takes the value `original` (the Secret as it was read) has under the same key, so
/// a binary value is carried through unchanged; with no such key the placeholder is plain text.
/// Any other object is left alone.
///
/// # Errors
///
/// [`SecretError::NotText`] for a value under `data` that is not a string (a number, a list), and
/// [`SecretError::Masked`] for one that is still the masked placeholder `(hidden)`: that is never
/// encoded as if it were the value.
pub fn from_decoded(object: &mut Value, original: Option<&Value>) -> Result<(), SecretError> {
    if !is_secret(object) {
        return Ok(());
    }
    let Some(data) = object.get_mut("data").and_then(Value::as_object_mut) else {
        return Ok(());
    };
    for (key, value) in data.iter_mut() {
        let Some(text) = value.as_str() else {
            if value.is_null() {
                continue;
            }
            return Err(SecretError::NotText(key.clone()));
        };
        if text == MASKED_VALUE {
            return Err(SecretError::Masked(key.clone()));
        }
        let kept = is_placeholder(text)
            .then(|| original_binary(original, key))
            .flatten();
        *value = Value::String(match kept {
            Some(encoded) => encoded,
            None => DecodedValue::Text(text.to_owned().into()).to_base64(),
        });
    }
    Ok(())
}

/// The original base64 of `key` when it is a binary value there.
fn original_binary(original: Option<&Value>, key: &str) -> Option<String> {
    let encoded = original?.get("data")?.get(key)?.as_str()?;
    match DecodedValue::from_base64(encoded) {
        DecodedValue::Binary { .. } => Some(encoded.to_owned()),
        DecodedValue::Text(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn secret() -> Value {
        json!({
            "apiVersion": "v1", "kind": "Secret",
            "metadata": {"name": "s", "annotations": {LAST_APPLIED: "{\"data\":{}}", "keep": "x"}},
            "type": "Opaque",
            "data": {"password": "aHVudGVyMg==", "blob": "//4=", "note": "bGluZSBvbmUKbGluZSB0d28="}
        })
    }

    #[test]
    fn decoded_shows_plaintext_and_placeholders() {
        let mut object = secret();
        assert!(to_decoded(&mut object));
        assert_eq!(object["data"]["password"], "hunter2");
        assert_eq!(object["data"]["blob"], "(binary, 2 bytes)");
        assert_eq!(object["data"]["note"], "line one\nline two");
        assert!(
            object["metadata"]["annotations"]
                .get(LAST_APPLIED)
                .is_none()
        );
        assert_eq!(object["metadata"]["annotations"]["keep"], "x");
    }

    #[test]
    fn editing_decoded_and_applying_encodes_base64_and_keeps_binary() {
        let original = secret();
        let mut buffer = original.clone();
        to_decoded(&mut buffer);
        buffer["data"]["password"] = json!("correct horse");
        buffer["data"]["added"] = json!("new value");
        from_decoded(&mut buffer, Some(&original)).unwrap();
        assert_eq!(buffer["data"]["password"], "Y29ycmVjdCBob3JzZQ==");
        assert_eq!(buffer["data"]["added"], "bmV3IHZhbHVl");
        assert_eq!(buffer["data"]["blob"], "//4=", "binary carried through");
        assert_eq!(buffer["data"]["note"], "bGluZSBvbmUKbGluZSB0d28=");
    }

    #[test]
    fn untouched_decoded_buffer_round_trips_to_the_same_data() {
        let original = secret();
        let mut buffer = original.clone();
        to_decoded(&mut buffer);
        from_decoded(&mut buffer, Some(&original)).unwrap();
        assert_eq!(buffer["data"], original["data"]);
    }

    #[test]
    fn a_placeholder_without_an_original_is_plain_text() {
        let mut buffer = json!({"apiVersion": "v1", "kind": "Secret",
            "data": {"x": "(binary, 2 bytes)"}});
        from_decoded(&mut buffer, None).unwrap();
        assert_eq!(
            DecodedValue::from_base64(buffer["data"]["x"].as_str().unwrap()).text(),
            Some("(binary, 2 bytes)")
        );
    }

    #[test]
    fn a_placeholder_over_a_text_original_is_plain_text() {
        let original = secret();
        let mut buffer = json!({"apiVersion": "v1", "kind": "Secret",
            "data": {"password": "(binary, 7 bytes)"}});
        from_decoded(&mut buffer, Some(&original)).unwrap();
        assert_ne!(buffer["data"]["password"], "aHVudGVyMg==");
    }

    #[test]
    fn a_non_string_value_is_refused() {
        let mut buffer = json!({"apiVersion": "v1", "kind": "Secret", "data": {"n": 5}});
        assert_eq!(
            from_decoded(&mut buffer, None),
            Err(SecretError::NotText("n".into()))
        );
    }

    #[test]
    fn a_masked_placeholder_is_never_encoded() {
        let mut buffer = json!({"apiVersion": "v1", "kind": "Secret",
            "data": {"password": "(hidden)", "user": "admin"}});
        assert_eq!(
            from_decoded(&mut buffer, None),
            Err(SecretError::Masked("password".into()))
        );
    }

    #[test]
    fn other_kinds_are_untouched() {
        let config = json!({"apiVersion": "v1", "kind": "ConfigMap", "data": {"a": "b"}});
        let mut copy = config.clone();
        assert!(!to_decoded(&mut copy));
        from_decoded(&mut copy, None).unwrap();
        assert_eq!(copy, config);
    }
}
