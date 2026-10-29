//! [`applied_change`]: created, configured or unchanged, from the object read before an apply and
//! the one the server returned. The one place that decides it for a real apply (a dry run uses
//! [`Change::between`] on the server's rehearsal).

use oxikube_domain::Resource;

use crate::apply::change::Change;

/// What a server-side apply did to an object, from `before` (read just before it, `None` when it
/// did not exist) and `after` (what the server returned).
///
/// * No object before: [`Change::Create`].
/// * The same `metadata.resourceVersion`: the server wrote nothing, [`Change::Unchanged`].
/// * A different `metadata.generation`: the spec changed, [`Change::Update`].
/// * Otherwise the content decides, ignoring the fields the server owns: a server-side apply that
///   only records the field manager (`managedFields`) bumps the version but changes nothing the
///   user wrote, which `kubectl apply` calls unchanged.
pub(crate) fn applied_change(before: Option<&Resource>, after: &Resource) -> Change {
    let Some(before) = before else {
        return Change::Create;
    };
    let version = |object: &Resource| object.meta.resource_version.clone();
    if version(before).is_some() && version(before) == version(after) {
        return Change::Unchanged;
    }
    let generation = |object: &Resource| {
        object
            .json()
            .pointer("/metadata/generation")
            .and_then(|generation| generation.as_i64())
    };
    if let (Some(was), Some(now)) = (generation(before), generation(after))
        && was != now
    {
        return Change::Update;
    }
    Change::between(Some(&before.to_value()), &after.to_value())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn object(version: &str, generation: i64, mode: &str) -> Resource {
        Resource::from_json(json!({
            "apiVersion": "v1", "kind": "ConfigMap",
            "metadata": {"name": "web", "namespace": "default",
                "resourceVersion": version, "generation": generation},
            "data": {"mode": mode}
        }))
        .unwrap()
    }

    #[test]
    fn nothing_before_is_a_create() {
        assert_eq!(applied_change(None, &object("1", 1, "a")), Change::Create);
    }

    #[test]
    fn the_same_version_is_unchanged_whatever_else() {
        let before = object("7", 1, "a");
        assert_eq!(
            applied_change(Some(&before), &object("7", 1, "a")),
            Change::Unchanged
        );
    }

    #[test]
    fn a_new_generation_is_configured() {
        let before = object("7", 1, "a");
        assert_eq!(
            applied_change(Some(&before), &object("8", 2, "a")),
            Change::Update
        );
    }

    #[test]
    fn a_new_version_with_new_content_is_configured() {
        let before = object("7", 1, "a");
        assert_eq!(
            applied_change(Some(&before), &object("8", 1, "b")),
            Change::Update
        );
    }

    #[test]
    fn a_new_version_that_only_records_the_manager_is_unchanged() {
        let before = object("7", 1, "a");
        let mut after = object("8", 1, "a").to_value();
        after["metadata"]["managedFields"] = json!([{"manager": "oxikube", "operation": "Apply"}]);
        let after = Resource::from_json(after).unwrap();
        assert_eq!(applied_change(Some(&before), &after), Change::Unchanged);
    }
}
