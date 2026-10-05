//! [`FeedObject`]: the domain [`Resource`] as a reflector store entry.
//!
//! Objects are converted once, at the edge, as each watch event arrives: kube's
//! `DynamicObject` becomes a domain `Resource` (with `managedFields` stripped by default),
//! and that is what both the reflector store and the consumer hold. No kube type is kept
//! past this point, and a relist diff compares domain objects directly.

use std::borrow::Cow;
use std::ops::Deref;

use kube::core::{ApiResource, DynamicObject};
use kube::runtime::reflector::Lookup;
use oxikube_domain::Resource;

use crate::resources::dynamic_json;

/// One object in a feed's reflector store. Derefs to the domain [`Resource`].
///
/// `Debug` prints identity only (through `Resource`'s own `Debug`), never the JSON, so a
/// Secret's data cannot reach a log.
#[derive(Clone, PartialEq, Debug)]
pub struct FeedObject(pub Resource);

impl FeedObject {
    /// Converts a watch or list item, consuming it. `None` when the server sent something
    /// that is not a valid object (no name); the caller skips it.
    pub(super) fn convert(
        item: DynamicObject,
        resource: &ApiResource,
        strip: bool,
    ) -> Option<Self> {
        Resource::from_json(dynamic_json(item, resource, strip))
            .ok()
            .map(Self)
    }

    /// The wrapped resource.
    pub fn into_inner(self) -> Resource {
        self.0
    }
}

impl Deref for FeedObject {
    type Target = Resource;

    fn deref(&self) -> &Resource {
        &self.0
    }
}

/// Store keys are (namespace, name), as for every kube reflector. The kind is fixed per
/// store, so the dynamic type carries nothing and the type-level names are empty.
impl Lookup for FeedObject {
    type DynamicType = ();

    fn kind(_: &()) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn group(_: &()) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn version(_: &()) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn plural(_: &()) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn name(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.0.name()))
    }

    fn namespace(&self) -> Option<Cow<'_, str>> {
        self.0.namespace().map(Cow::Borrowed)
    }

    fn resource_version(&self) -> Option<Cow<'_, str>> {
        self.0.meta.resource_version.as_deref().map(Cow::Borrowed)
    }

    fn uid(&self) -> Option<Cow<'_, str>> {
        self.0.meta.uid.as_deref().map(Cow::Borrowed)
    }
}
