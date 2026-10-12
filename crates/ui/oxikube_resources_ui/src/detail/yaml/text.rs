//! The text of the YAML tab: a pure function over the object, nothing from GPUI.
//!
//! The cached object is never touched: the function works on a copy (one per object version,
//! when the tab is shown), strips what the user asked to hide, masks what must not be shown, and
//! serialises with the domain's YAML writer (`serde-saphyr`, which quotes `y`, `n`, `on`, `1e3`
//! and the like so they read back as strings).

use std::collections::BTreeSet;

use oxikube_domain::Resource;
use oxikube_domain::secret::DecodedSecret;

use crate::detail::model::{HIDDEN, mask_secret_with};

/// What the YAML tab shows of an object.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct YamlOptions {
    /// Keep `metadata.managedFields` (hidden by default, as `kubectl get -o yaml` hides it and
    /// k9s's `m` toggles it).
    pub managed_fields: bool,
}

/// The YAML `resource` is shown as.
///
/// * `metadata.managedFields` is removed unless `options.managed_fields`.
/// * A Secret's `data` and `stringData` values are replaced by [`HIDDEN`] (keys stay, in their
///   order), and the `last-applied-configuration` annotation, which embeds them, is dropped (it
///   does not appear at all, not even as a placeholder). This holds
///   for whatever the store delivered: a Secret that arrived whole still shows no value.
///
/// Copy and save write exactly this text, so a masked view is saved masked.
///
/// # Errors
///
/// The serialiser's message, when it fails (it does not for any JSON a cluster serves).
pub fn yaml_text(resource: &Resource, options: YamlOptions) -> Result<String, String> {
    let mut shown = resource.clone();
    mask_secret_with(&mut shown, Some(HIDDEN));
    if !options.managed_fields {
        shown.strip_managed_fields();
    }
    shown.to_yaml().map_err(|error| error.to_string())
}

/// [`yaml_text`] with some of a Secret's values shown as text (E12-S11): each key of `keys` (every
/// key when `None`) that `decoded` holds shows its plaintext, a binary value its size
/// (`(binary, 12 bytes)`); the other keys stay [`HIDDEN`]. The text holds plaintext: it lives in
/// memory, and the detail refuses to save it to a file.
///
/// # Errors
///
/// The serialiser's message, when it fails (it does not for any JSON a cluster serves).
pub fn yaml_text_revealed(
    resource: &Resource,
    options: YamlOptions,
    decoded: &DecodedSecret,
    keys: Option<&BTreeSet<String>>,
) -> Result<String, String> {
    let mut shown = resource.clone();
    mask_secret_with(&mut shown, Some(HIDDEN));
    if !options.managed_fields {
        shown.strip_managed_fields();
    }
    shown.edit_json(|json| {
        let Some(data) = json.get_mut("data").and_then(|data| data.as_object_mut()) else {
            return;
        };
        for (key, value) in data.iter_mut() {
            if keys.is_some_and(|keys| !keys.contains(key)) {
                continue;
            }
            let Some(decoded) = decoded.get(key) else {
                continue;
            };
            *value = decoded.shown().into();
        }
    });
    shown.to_yaml().map_err(|error| error.to_string())
}

/// Whether `resource` has `metadata.managedFields` (the toggle has something to show).
pub fn has_managed_fields(resource: &Resource) -> bool {
    resource
        .get("/metadata/managedFields")
        .is_some_and(|fields| !fields.is_null())
}
