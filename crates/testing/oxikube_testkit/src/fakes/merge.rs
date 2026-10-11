//! JSON merge patch (RFC 7386), for [`FakeResourcePort`](super::FakeResourcePort)'s `patch`.

use serde_json::Value;

/// Applies the merge patch `patch` to `target`: objects merge key by key, a `null` removes a key,
/// anything else (arrays included) replaces the value.
pub(super) fn merge_patch(target: &mut Value, patch: &Value) {
    let Value::Object(changes) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(Default::default());
    }
    let Some(map) = target.as_object_mut() else {
        return;
    };
    for (key, change) in changes {
        if change.is_null() {
            map.remove(key);
        } else {
            merge_patch(map.entry(key.clone()).or_insert(Value::Null), change);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::merge_patch;

    #[test]
    fn objects_merge_nulls_remove_and_arrays_replace() {
        let mut live = json!({"spec": {"suspend": false, "tags": [1, 2], "keep": 1}, "a": 1});
        merge_patch(
            &mut live,
            &json!({"spec": {"suspend": true, "tags": [3], "gone": null}, "a": null}),
        );
        assert_eq!(
            live,
            json!({"spec": {"suspend": true, "tags": [3], "keep": 1}})
        );
    }
}
