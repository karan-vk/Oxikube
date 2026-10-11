//! JSON merge patch (RFC 7386) and the strategic merge patch's list rule, for
//! [`FakeResourcePort`](super::FakeResourcePort)'s `patch`.

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

/// Applies the strategic merge patch `patch` to `target`, as far as the pod template's lists need:
/// like [`merge_patch`], except that a list of objects that all carry a `name` is merged by name
/// (`patchMergeKey: name` of `containers`, `initContainers`, `env`, `volumes`): an entry with a
/// known name is merged into that entry, a new name is appended, and unnamed entries of the live
/// list stay. Any other list is replaced.
pub(super) fn strategic_merge_patch(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(live), Value::Object(changes)) => {
            for (key, change) in changes {
                if change.is_null() {
                    live.remove(key);
                } else {
                    strategic_merge_patch(live.entry(key.clone()).or_insert(Value::Null), change);
                }
            }
        }
        (Value::Array(live), Value::Array(changes))
            if keyed_by_name(live) && keyed_by_name(changes) =>
        {
            for change in changes {
                let name = change.get("name");
                match live.iter_mut().find(|entry| entry.get("name") == name) {
                    Some(entry) => strategic_merge_patch(entry, change),
                    None => live.push(change.clone()),
                }
            }
        }
        (target, patch) => merge_patch(target, patch),
    }
}

/// Whether every entry of `list` is an object with a string `name` (and `list` is not empty).
fn keyed_by_name(list: &[Value]) -> bool {
    !list.is_empty()
        && list
            .iter()
            .all(|entry| entry.get("name").is_some_and(Value::is_string))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{merge_patch, strategic_merge_patch};

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

    #[test]
    fn a_strategic_patch_merges_named_lists_by_name_and_keeps_the_rest() {
        let mut live = json!({"spec": {"containers": [
            {"name": "web", "image": "nginx:1", "ports": [80]},
            {"name": "sidecar", "image": "envoy:1"},
        ], "ports": [1, 2]}});
        strategic_merge_patch(
            &mut live,
            &json!({"spec": {"containers": [{"name": "web", "image": "nginx:2"}], "ports": [3]}}),
        );
        assert_eq!(
            live,
            json!({"spec": {"containers": [
                {"name": "web", "image": "nginx:2", "ports": [80]},
                {"name": "sidecar", "image": "envoy:1"},
            ], "ports": [3]}})
        );
    }
}
