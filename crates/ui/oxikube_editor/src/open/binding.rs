//! [`ResourceBinding`]: what a manifest editor opened from a live object holds besides its
//! buffer: which object, how it was opened, what the text shows of it and how far the load is.
//!
//! The object itself is kept (an `Arc`, in memory only) so the toggles render it again without
//! another `get`. Revealed Secret values are therefore only ever in this binding and in the
//! buffer; neither is serialised (the editor item is not saved with the layout), logged
//! (`Resource`'s `Debug` prints its identity only) or written anywhere.

use std::sync::Arc;

use gpui::SharedString;
use oxikube_domain::Resource;
use oxikube_domain::ids::ResourceRef;
use oxikube_ports::ResourceReader;

use super::render::{RenderOptions, RenderedManifest};

/// How the user asked for the object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenMode {
    /// "Edit" (`resource::Edit`): the buffer takes typing, unless the cluster is read-only.
    Edit,
    /// "View YAML in Editor" (`resource::ViewYamlInEditor`): the buffer is read-only.
    View,
}

/// How far the editor is with reading the object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourceLoad {
    /// The fresh `get` is in flight; the buffer is empty.
    Loading,
    /// The buffer holds the object.
    Ready,
    /// The object could not be read (the message is shown in the toolbar).
    Failed(String),
}

/// What opens a [`ManifestEditor`](crate::view::ManifestEditor) on an object.
#[derive(Clone)]
pub struct ResourceOpen {
    /// The object.
    pub target: ResourceRef,
    /// Editable or read-only.
    pub mode: OpenMode,
    /// The cluster's reader for the fresh `get`; `None` while it is not connected.
    pub reader: Option<Arc<dyn ResourceReader>>,
    /// Whether the cluster is in read-only mode (an edit tab then opens read-only, with a
    /// notice).
    pub cluster_read_only: bool,
}

/// The object an editor was opened on, and what its text shows of it.
pub struct ResourceBinding {
    pub(crate) target: ResourceRef,
    pub(crate) mode: OpenMode,
    pub(crate) options: RenderOptions,
    /// The options of the text in the buffer (`options` runs ahead while a render is in flight).
    pub(crate) shown_options: RenderOptions,
    pub(crate) load: ResourceLoad,
    pub(crate) cluster_read_only: bool,
    pub(crate) reader: Option<Arc<dyn ResourceReader>>,
    /// The object as read, values included: the toggles render it again.
    pub(crate) resource: Option<Arc<Resource>>,
    /// How many `managedFields` entries the text leaves out.
    hidden_managed_fields: usize,
    pub(crate) notice: Option<SharedString>,
    /// Counts the renders asked for; a result for an older one is dropped.
    pub(crate) generation: u64,
}

impl ResourceBinding {
    pub(crate) fn new(open: ResourceOpen) -> Self {
        Self {
            target: open.target,
            mode: open.mode,
            options: RenderOptions::default(),
            shown_options: RenderOptions::default(),
            load: ResourceLoad::Loading,
            cluster_read_only: open.cluster_read_only,
            reader: open.reader,
            resource: None,
            hidden_managed_fields: 0,
            notice: None,
            generation: 0,
        }
    }

    /// The object.
    pub fn target(&self) -> &ResourceRef {
        &self.target
    }

    /// The tab title: `Deployment/web`.
    pub fn title(&self) -> SharedString {
        format!("{}/{}", self.target.gvk.kind, self.target.name).into()
    }

    /// The title with the namespace: `Deployment/web · namespace default`.
    pub fn subject(&self) -> SharedString {
        match &self.target.namespace {
            Some(namespace) => format!("{} · namespace {namespace}", self.title()).into(),
            None => self.title(),
        }
    }

    /// What the text shows of the object.
    pub fn options(&self) -> RenderOptions {
        self.options
    }

    /// How far the load is.
    pub fn load(&self) -> &ResourceLoad {
        &self.load
    }

    /// A line for the toolbar: why the buffer is read-only, what failed.
    pub fn notice(&self) -> Option<&SharedString> {
        self.notice.as_ref()
    }

    /// Whether the object is a Secret (the reveal toggle has something to do).
    pub fn is_secret(&self) -> bool {
        self.target.gvk.group.is_empty() && &*self.target.gvk.kind == "Secret"
    }

    /// How many `managedFields` entries the text leaves out.
    pub fn hidden_managed_fields(&self) -> usize {
        self.hidden_managed_fields
    }

    /// Whether the object has `managedFields` at all (the toggle has something to show): leaves
    /// out some now, or shows them.
    pub fn has_managed_fields(&self) -> bool {
        self.options.managed_fields
            || self.hidden_managed_fields > 0
            || self
                .resource
                .as_ref()
                .is_some_and(|r| r.get("/metadata/managedFields").is_some())
    }

    /// Whether the text masks Secret values now (a placeholder left in it is an error).
    pub fn masking(&self) -> bool {
        self.is_secret() && !self.options.reveal_secrets
    }

    pub(crate) fn rendered(&mut self, rendered: &RenderedManifest) {
        self.hidden_managed_fields = rendered.hidden_managed_fields();
        self.shown_options = self.options;
        self.load = ResourceLoad::Ready;
    }
}
