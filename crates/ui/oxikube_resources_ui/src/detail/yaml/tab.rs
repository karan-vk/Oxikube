//! The YAML tab's state and the view's methods over it.

use std::sync::Arc;

use gpui::{AppContext as _, Context, Entity, Task};
use oxikube_app::store::StoreObject;
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_ui::code_view::{CodeView, Look};

use super::text::{YamlOptions, has_managed_fields, yaml_text};
use crate::detail::state::FullState;
use crate::detail::tabs::DetailTab;
use crate::detail::view::DetailView;

/// What the text was made for: the object's version and the managedFields choice. The text is
/// made again only when this changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::detail) struct YamlKey {
    pub(in crate::detail) version: Option<Arc<str>>,
    pub(in crate::detail) managed_fields: bool,
}

/// The text for one [`YamlKey`].
pub(in crate::detail) struct YamlText {
    pub(in crate::detail) key: YamlKey,
    /// The text, or why there is none.
    pub(in crate::detail) result: Result<Arc<str>, String>,
    /// Whether the object has `managedFields` (the toggle does something).
    pub(in crate::detail) has_managed_fields: bool,
}

/// The YAML tab of a [`DetailView`].
#[derive(Default)]
pub(in crate::detail) struct YamlTab {
    /// `metadata.managedFields` shown (hidden by default).
    pub(in crate::detail) managed_fields: bool,
    pub(in crate::detail) text: Option<YamlText>,
    /// The key of the text being made off the UI thread.
    pub(in crate::detail) making: Option<YamlKey>,
    /// Makes that text; a newer request replaces (and so cancels) it.
    pub(in crate::detail) task: Option<Task<()>>,
    /// The read-only view of the text, made with the tab's first text.
    pub(in crate::detail) view: Option<Entity<CodeView>>,
    /// How many texts were made (one per object version and managedFields choice).
    #[cfg(test)]
    pub(in crate::detail) made: usize,
}

/// The complete object, shared with the background thread that writes it as YAML (no copy on
/// the UI thread).
pub(in crate::detail) enum Complete {
    Read(Arc<Resource>),
    Store(Arc<StoreObject>),
}

impl Complete {
    pub(in crate::detail) fn resource(&self) -> Option<&Resource> {
        match self {
            Complete::Read(resource) => Some(resource),
            Complete::Store(object) => match object.as_ref() {
                StoreObject::Resource(resource) => Some(resource),
                StoreObject::Row(_) => None,
            },
        }
    }
}

/// Writes `object` as the tab shows it (on a background thread).
fn make(object: &Complete, key: YamlKey) -> YamlText {
    let Some(resource) = object.resource() else {
        return YamlText {
            key,
            result: Err("The object is not known in full.".to_owned()),
            has_managed_fields: false,
        };
    };
    let options = YamlOptions {
        managed_fields: key.managed_fields,
    };
    YamlText {
        has_managed_fields: has_managed_fields(resource),
        result: yaml_text(resource, options)
            .map(Arc::<str>::from)
            .map_err(|error| format!("The object cannot be written as YAML: {error}")),
        key,
    }
}

impl DetailView {
    /// The complete object (the full read when there is one, else the store's object when it is
    /// whole), as a handle the background thread can hold. `None` for a metadata-only or Table
    /// object until its full read lands.
    pub(in crate::detail) fn complete_object(&self) -> Option<Complete> {
        if let FullState::Loaded(resource) = &self.full {
            return Some(Complete::Read(resource.clone()));
        }
        let object = self.object.clone()?;
        matches!(object.as_ref(), StoreObject::Resource(r) if !r.is_partial())
            .then_some(Complete::Store(object))
    }

    /// Makes the YAML text for the current object, off the UI thread, when the tab is shown and
    /// the text is not already the one for this version. The text on screen stays until the new
    /// one is ready.
    pub(in crate::detail) fn refresh_yaml(&mut self, cx: &mut Context<Self>) {
        if self.tab != DetailTab::Yaml {
            return;
        }
        let managed_fields = self.yaml.managed_fields;
        let Some(object) = self.complete_object() else {
            // The object is no longer known in full (its re-read failed, or it is gone): the
            // old text is not the current object, so neither shown nor copied nor saved.
            self.stop_making_yaml();
            if self.yaml.text.take().is_some()
                && let Some(view) = &self.yaml.view
            {
                view.update(cx, |view, cx| view.clear(cx));
            }
            return;
        };
        let key = YamlKey {
            version: object
                .resource()
                .and_then(|resource| resource.meta.resource_version.clone()),
            managed_fields,
        };
        if self.yaml.text.as_ref().is_some_and(|t| t.key == key) {
            // Back to the text on screen (a toggle undone before its text was made).
            self.stop_making_yaml();
            return;
        }
        if self.yaml.making.as_ref() == Some(&key) {
            return;
        }
        if self.yaml.view.is_none() {
            self.yaml.view = Some(cx.new(|cx| CodeView::new(Look::YAML, cx)));
        }
        self.yaml.making = Some(key.clone());
        self.yaml.task = Some(cx.spawn(async move |this, cx| {
            let made = cx
                .background_executor()
                .spawn(async move { make(&object, key) })
                .await;
            this.update(cx, |view, cx| view.yaml_made(made, cx)).ok();
        }));
    }

    /// Drops the text in flight (and so cancels making it).
    fn stop_making_yaml(&mut self) {
        self.yaml.making = None;
        self.yaml.task = None;
    }

    /// A text made off the UI thread: shown, unless a newer request replaced it.
    fn yaml_made(&mut self, made: YamlText, cx: &mut Context<Self>) {
        if self.yaml.making.as_ref() != Some(&made.key) {
            return;
        }
        self.yaml.making = None;
        #[cfg(test)]
        {
            self.yaml.made += 1;
        }
        if let (Ok(text), Some(view)) = (&made.result, &self.yaml.view) {
            let text = text.clone();
            view.update(cx, |view, cx| view.set_text(text, cx));
        }
        self.yaml.text = Some(made);
        cx.notify();
    }

    /// The YAML the tab shows (what copy and save write), once the object is known in full.
    pub fn yaml(&self) -> Option<&str> {
        self.yaml.text.as_ref()?.result.as_deref().ok()
    }

    /// Whether `metadata.managedFields` is shown in the YAML.
    pub fn managed_fields_shown(&self) -> bool {
        self.yaml.managed_fields
    }

    /// The file name the YAML is saved under by default: `<name>.yaml`.
    pub fn yaml_file_name(&self) -> String {
        format!("{}.yaml", self.target.name)
    }

    /// Shows or hides `managedFields`; the text is made again when the tab is shown.
    pub fn set_managed_fields(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.yaml.managed_fields == shown {
            return;
        }
        self.yaml.managed_fields = shown;
        self.refresh_yaml(cx);
        cx.notify();
    }

    /// Flips the managedFields choice (`resource::ToggleManagedFields`).
    pub fn toggle_managed_fields(&mut self, cx: &mut Context<Self>) {
        self.set_managed_fields(!self.yaml.managed_fields, cx);
    }

    /// The toggle button: sends `resource::ToggleManagedFields`.
    pub fn request_toggle_managed_fields(&mut self, cx: &mut Context<Self>) {
        let command = Command::ResourceToggleManagedFields {
            target: self.target.clone(),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// The copy button: sends `resource::CopyYaml`.
    pub fn request_copy_yaml(&mut self, cx: &mut Context<Self>) {
        let command = Command::ResourceCopyYaml {
            target: self.target.clone(),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// The save button: sends `resource::SaveYaml`.
    pub fn request_save_yaml(&mut self, cx: &mut Context<Self>) {
        let command = Command::ResourceSaveYaml {
            target: self.target.clone(),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }
}
