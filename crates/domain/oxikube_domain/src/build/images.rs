//! `kubectl set image` as pure functions: the containers of a pod template, the checks on the
//! images a user typed, and the strategic merge patch that changes only the named containers.
//!
//! A plain merge patch would replace the whole `containers` array, so the patch is a *strategic*
//! merge patch: the API server merges the list by container name (`patchMergeKey: name`) for
//! the Deployment, StatefulSet and DaemonSet pod templates alike, and leaves every other
//! container and every other field of a named container alone. Init containers are a separate
//! list (`initContainers`) with the same merge key.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::Resource;
use crate::command::ImageChange;
use crate::{OxiError, OxiResult};

/// A container of a workload's pod template and the image it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSlot {
    /// The container's name.
    pub name: Arc<str>,
    /// Its image reference (`spec.template.spec.containers[].image`); empty if the object has none.
    pub image: Arc<str>,
    /// Whether it is an init container.
    pub init: bool,
}

/// One planned change: container `container` runs `to` instead of `from`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageEdit {
    /// The container's name.
    pub container: Arc<str>,
    /// Whether it is an init container.
    pub init: bool,
    /// The image it runs now.
    pub from: Arc<str>,
    /// The image it will run.
    pub to: Arc<str>,
}

impl ImageEdit {
    /// `web: nginx:1.26 -> nginx:1.27`, the line a diff shows and an audit record carries.
    pub fn describe(&self) -> String {
        format!("{}: {} -> {}", self.container, self.from, self.to)
    }
}

/// The containers of `workload`'s pod template, init containers first (`spec.template.spec`).
/// Ephemeral containers are not part of a template and are not listed.
///
/// # Errors
///
/// `Validation` when the object has no pod template.
pub fn template_images(workload: &Resource) -> OxiResult<Vec<ImageSlot>> {
    let Some(pod_spec) = workload.get("/spec/template/spec") else {
        return Err(OxiError::validation(format!(
            "{} has no pod template",
            workload.name()
        )));
    };
    let mut slots = Vec::new();
    for (key, init) in [("initContainers", true), ("containers", false)] {
        let Some(list) = pod_spec.get(key).and_then(|list| list.as_array()) else {
            continue;
        };
        for container in list {
            let text = |field: &str| {
                container
                    .get(field)
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
            };
            slots.push(ImageSlot {
                name: Arc::from(text("name")),
                image: Arc::from(text("image")),
                init,
            });
        }
    }
    Ok(slots)
}

/// Why `image` cannot be set, in words for the field: empty, or with a space in it. `None` for an
/// image the API server may accept (it checks the reference itself; there is no registry lookup).
pub fn image_problem(image: &str) -> Option<&'static str> {
    let image = image.trim();
    if image.is_empty() {
        Some("Enter an image.")
    } else if image.chars().any(|c| c.is_whitespace() || c.is_control()) {
        Some("An image cannot contain spaces.")
    } else {
        None
    }
}

/// The edits `changes` make to a template whose containers are `current`.
///
/// Surrounding blanks of an image are ignored. A change to the image a container already runs is
/// dropped (setting an image is idempotent), so what is returned is exactly what the patch will
/// alter.
///
/// # Errors
///
/// `Validation` when `changes` is empty, names a container twice or one the template does not
/// have, holds an image [`image_problem`] refuses, or changes nothing at all.
pub fn plan_image_changes(
    current: &[ImageSlot],
    changes: &[ImageChange],
) -> OxiResult<Vec<ImageEdit>> {
    if changes.is_empty() {
        return Err(OxiError::validation("Name a container and an image."));
    }
    let mut edits: Vec<ImageEdit> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for change in changes {
        let name = change.container.as_str();
        if seen.contains(&name) {
            return Err(OxiError::validation(format!(
                "Container {name} is named twice."
            )));
        }
        seen.push(name);
        let Some(slot) = current.iter().find(|slot| &*slot.name == name) else {
            return Err(OxiError::validation(format!(
                "The pod template has no container named {name}."
            )));
        };
        if let Some(problem) = image_problem(&change.image) {
            return Err(OxiError::validation(format!("{name}: {problem}")));
        }
        let to = change.image.trim();
        if to != &*slot.image {
            edits.push(ImageEdit {
                container: slot.name.clone(),
                init: slot.init,
                from: slot.image.clone(),
                to: Arc::from(to),
            });
        }
    }
    if edits.is_empty() {
        return Err(OxiError::validation(
            "Those images are what the containers run already.",
        ));
    }
    Ok(edits)
}

/// The strategic merge patch body that sets the images of `edits`: only the named containers, in
/// `containers` or `initContainers` as each one is, with nothing else of the template.
pub fn set_image_patch(edits: &[ImageEdit]) -> Value {
    let list = |init: bool| -> Vec<Value> {
        edits
            .iter()
            .filter(|edit| edit.init == init)
            .map(|edit| json!({"name": &*edit.container, "image": &*edit.to}))
            .collect()
    };
    let mut pod_spec = serde_json::Map::new();
    for (key, init) in [("initContainers", true), ("containers", false)] {
        let entries = list(init);
        if !entries.is_empty() {
            pod_spec.insert(key.to_owned(), Value::Array(entries));
        }
    }
    json!({"spec": {"template": {"spec": Value::Object(pod_spec)}}})
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::ErrorKind;

    fn deployment() -> Resource {
        Resource::from_json(json!({
            "apiVersion": "apps/v1", "kind": "Deployment",
            "metadata": {"name": "web", "namespace": "shop"},
            "spec": {"template": {"spec": {
                "initContainers": [{"name": "migrate", "image": "app:1"}],
                "containers": [
                    {"name": "web", "image": "nginx:1.26"},
                    {"name": "sidecar", "image": "envoy:1"}
                ],
            }}},
        }))
        .unwrap()
    }

    #[test]
    fn the_template_lists_init_containers_first() {
        let slots = template_images(&deployment()).unwrap();
        let listed: Vec<_> = slots
            .iter()
            .map(|s| (&*s.name, &*s.image, s.init))
            .collect();
        assert_eq!(
            listed,
            [
                ("migrate", "app:1", true),
                ("web", "nginx:1.26", false),
                ("sidecar", "envoy:1", false)
            ]
        );
        let bare = Resource::from_json(json!({
            "apiVersion": "apps/v1", "kind": "Deployment", "metadata": {"name": "x"}, "spec": {},
        }))
        .unwrap();
        assert_eq!(
            template_images(&bare).unwrap_err().kind(),
            ErrorKind::Validation
        );
    }

    #[test]
    fn a_change_names_one_container_and_keeps_the_others_out_of_the_patch() {
        let slots = template_images(&deployment()).unwrap();
        let edits = plan_image_changes(&slots, &[ImageChange::new("web", " nginx:1.27 ")]).unwrap();
        assert_eq!(edits[0].describe(), "web: nginx:1.26 -> nginx:1.27");
        assert_eq!(
            set_image_patch(&edits),
            json!({"spec": {"template": {"spec": {
                "containers": [{"name": "web", "image": "nginx:1.27"}]
            }}}})
        );
    }

    #[test]
    fn an_init_container_goes_in_its_own_list_and_several_changes_share_a_patch() {
        let slots = template_images(&deployment()).unwrap();
        let edits = plan_image_changes(
            &slots,
            &[
                ImageChange::new("migrate", "app:2"),
                ImageChange::new("sidecar", "envoy:2"),
            ],
        )
        .unwrap();
        assert_eq!(
            set_image_patch(&edits),
            json!({"spec": {"template": {"spec": {
                "initContainers": [{"name": "migrate", "image": "app:2"}],
                "containers": [{"name": "sidecar", "image": "envoy:2"}],
            }}}})
        );
    }

    #[test]
    fn unchanged_images_are_dropped_and_a_plan_of_nothing_is_refused() {
        let slots = template_images(&deployment()).unwrap();
        let edits = plan_image_changes(
            &slots,
            &[
                ImageChange::new("web", "nginx:1.26"),
                ImageChange::new("sidecar", "envoy:2"),
            ],
        )
        .unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(&*edits[0].container, "sidecar");
        let error =
            plan_image_changes(&slots, &[ImageChange::new("web", "nginx:1.26")]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Validation);
    }

    #[test]
    fn bad_input_is_a_validation_error_in_words() {
        let slots = template_images(&deployment()).unwrap();
        for (changes, needle) in [
            (vec![], "Name a container"),
            (vec![ImageChange::new("web", "")], "Enter an image"),
            (vec![ImageChange::new("web", "   ")], "Enter an image"),
            (
                vec![ImageChange::new("web", "nginx 1")],
                "cannot contain spaces",
            ),
            (
                vec![ImageChange::new("nope", "x:1")],
                "no container named nope",
            ),
            (
                vec![
                    ImageChange::new("web", "a:1"),
                    ImageChange::new("web", "b:1"),
                ],
                "named twice",
            ),
        ] {
            let error = plan_image_changes(&slots, &changes).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Validation, "{changes:?}");
            assert!(error.message().contains(needle), "{}", error.message());
        }
    }
}
