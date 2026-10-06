//! The YAML tab's state and the view's methods over it.

use std::sync::Arc;

use gpui::{Context, Entity};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_ui::editor::EditorState;

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
    /// The editor, made the first time the tab is drawn.
    pub(in crate::detail) editor: Option<Entity<EditorState>>,
    /// The key of the text the editor holds.
    pub(in crate::detail) pushed: Option<YamlKey>,
}

impl DetailView {
    /// The complete object: the full read when there is one, else the store's object when it is
    /// whole. `None` for a metadata-only or Table object until its full read lands.
    pub(in crate::detail) fn complete_resource(&self) -> Option<&Resource> {
        if let FullState::Loaded(resource) = &self.full {
            return Some(resource.as_ref());
        }
        match self.object.as_deref() {
            Some(oxikube_app::store::StoreObject::Resource(resource)) if !resource.is_partial() => {
                Some(resource)
            }
            _ => None,
        }
    }

    /// Makes the YAML text for the current object when the tab is shown and the text is not
    /// already the one for this version. Returns whether the text changed.
    pub(in crate::detail) fn refresh_yaml(&mut self) -> bool {
        if self.tab != DetailTab::Yaml {
            return false;
        }
        let managed_fields = self.yaml.managed_fields;
        let made = self.complete_resource().and_then(|resource| {
            let key = YamlKey {
                version: resource.meta.resource_version.clone(),
                managed_fields,
            };
            if self.yaml.text.as_ref().is_some_and(|t| t.key == key) {
                return None;
            }
            let result = yaml_text(resource, YamlOptions { managed_fields })
                .map(Arc::<str>::from)
                .map_err(|error| format!("The object cannot be written as YAML: {error}"));
            Some(YamlText {
                key,
                result,
                has_managed_fields: has_managed_fields(resource),
            })
        });
        match made {
            Some(text) => {
                self.yaml.text = Some(text);
                true
            }
            None => false,
        }
    }

    /// The YAML the tab shows (what copy and save write), once the object is known in full.
    pub fn yaml(&self) -> Option<&str> {
        match &self.yaml.text.as_ref()?.result {
            Ok(text) => Some(text),
            Err(_) => None,
        }
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
        self.refresh_yaml();
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
