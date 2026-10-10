//! [`EditorSettings`]: the resolved `editor` settings, their validation and how a store is read.

use std::sync::OnceLock;

use gpui::App;
use oxikube_domain::ids::ClusterId;
use oxikube_settings::{Settings, SettingsLocation, SettingsStore};
use oxikube_ui::editor::EditorFont;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::content::EditorSettingsContent;

/// The key of the section in `settings.json`.
pub const KEY: &str = "editor";
/// The smallest `editor.tab_size`.
pub const MIN_TAB_SIZE: usize = 1;
/// The largest `editor.tab_size`.
pub const MAX_TAB_SIZE: usize = 16;
/// `editor.tab_size` when it says nothing valid.
pub const DEFAULT_TAB_SIZE: usize = 2;
/// The smallest `editor.font_size`, in points.
pub const MIN_FONT_SIZE: f32 = 6.0;
/// The largest `editor.font_size`, in points.
pub const MAX_FONT_SIZE: f32 = 72.0;
/// The longest `editor.field_manager`, as the API server accepts it.
pub const MAX_FIELD_MANAGER_LEN: usize = 128;
/// `editor.field_manager` when it says nothing valid.
pub const DEFAULT_FIELD_MANAGER: &str = "oxikube";

/// The resolved `editor` settings: every field has a value, invalid input already replaced by
/// the default (and named in [`EditorSettings::problems`]).
#[derive(Clone, Debug, PartialEq)]
pub struct EditorSettings {
    /// Spaces one indent is, within [`MIN_TAB_SIZE`] ..= [`MAX_TAB_SIZE`].
    pub tab_size: usize,
    /// The font family; `None` is the theme's monospace font.
    pub font_family: Option<String>,
    /// The font size in points at 100 % zoom, within [`MIN_FONT_SIZE`] ..= [`MAX_FONT_SIZE`];
    /// `None` is the theme's monospace size.
    pub font_size: Option<f32>,
    /// Wrap long lines.
    pub soft_wrap: bool,
    /// Show `metadata.managedFields` of a live object (E10-S06 reads it).
    pub show_managed_fields: bool,
    /// Mask the values of a Secret's `data` when the object is opened (E10-S06 reads it).
    pub mask_secrets: bool,
    /// Check the manifest against the cluster's schemas.
    pub validation: bool,
    /// The server-side apply field manager (E10-S08 reads it at apply time): non-empty, at most
    /// [`MAX_FIELD_MANAGER_LEN`] characters, no whitespace.
    pub field_manager: String,
    /// What was wrong in the layers read, one sentence each, naming the key and the default used
    /// instead. Never holds a value from the file. Empty for a valid block.
    pub problems: Vec<String>,
}

impl Default for EditorSettings {
    /// What the shipped `default.json` says.
    fn default() -> Self {
        Self::from_content(EditorSettingsContent::default())
    }
}

impl Settings for EditorSettings {
    const KEY: Option<&'static str> = Some(KEY);
    type Content = EditorSettingsContent;

    fn from_content(content: EditorSettingsContent) -> Self {
        let mut problems = Vec::new();
        let tab_size = match content.tab_size {
            None => DEFAULT_TAB_SIZE,
            Some(size) if (MIN_TAB_SIZE as i64..=MAX_TAB_SIZE as i64).contains(&size) => {
                size as usize
            }
            Some(_) => {
                problems.push(format!(
                    "editor.tab_size must be {MIN_TAB_SIZE} to {MAX_TAB_SIZE}: using {DEFAULT_TAB_SIZE}"
                ));
                DEFAULT_TAB_SIZE
            }
        };
        let font_size = match content.font_size {
            None => None,
            Some(size)
                if size.is_finite()
                    && (f64::from(MIN_FONT_SIZE)..=f64::from(MAX_FONT_SIZE)).contains(&size) =>
            {
                Some(size as f32)
            }
            Some(_) => {
                problems.push(format!(
                    "editor.font_size must be {MIN_FONT_SIZE} to {MAX_FONT_SIZE}: using the theme's size"
                ));
                None
            }
        };
        let field_manager = match content.field_manager {
            None => DEFAULT_FIELD_MANAGER.to_owned(),
            Some(name) if valid_field_manager(&name) => name,
            Some(_) => {
                problems.push(format!(
                    "editor.field_manager must be 1 to {MAX_FIELD_MANAGER_LEN} characters without \
                     whitespace: using \"{DEFAULT_FIELD_MANAGER}\""
                ));
                DEFAULT_FIELD_MANAGER.to_owned()
            }
        };
        Self {
            tab_size,
            font_family: content
                .font_family
                .map(|family| family.trim().to_owned())
                .filter(|family| !family.is_empty()),
            font_size,
            soft_wrap: content.soft_wrap.unwrap_or(false),
            show_managed_fields: content.show_managed_fields.unwrap_or(false),
            mask_secrets: content.mask_secrets.unwrap_or(true),
            validation: content.validation.unwrap_or(true),
            field_manager,
            problems,
        }
    }

    /// A field of the wrong type (a number for `field_manager`, a string for `tab_size`) fails the
    /// block as a whole; this keeps every other field and puts the default in the wrong one.
    fn salvage(merged: &Value, _base: &Self) -> Option<Self> {
        let section = merged.get(KEY)?.as_object()?;
        // Parse each field on its own; one that does not parse is left unset and recorded.
        let mut wrong = Vec::new();
        let content = EditorSettingsContent {
            tab_size: lenient(section, "tab_size", &mut wrong),
            font_family: lenient(section, "font_family", &mut wrong),
            font_size: lenient(section, "font_size", &mut wrong),
            soft_wrap: lenient(section, "soft_wrap", &mut wrong),
            show_managed_fields: lenient(section, "show_managed_fields", &mut wrong),
            mask_secrets: lenient(section, "mask_secrets", &mut wrong),
            validation: lenient(section, "validation", &mut wrong),
            field_manager: lenient(section, "field_manager", &mut wrong),
        };
        let mut settings = Self::from_content(content);
        settings.problems.extend(
            wrong
                .into_iter()
                .map(|key| format!("editor.{key} has the wrong type: using the default")),
        );
        Some(settings)
    }
}

oxikube_settings::register_settings!(EditorSettings);

/// Whether `name` is usable as a field manager.
pub fn valid_field_manager(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_FIELD_MANAGER_LEN
        && !name.chars().any(char::is_whitespace)
}

/// `section[key]` as a `T`; `None` when it is absent, `null`, or of another type (then `key` is
/// added to `wrong`).
fn lenient<T: DeserializeOwned>(
    section: &serde_json::Map<String, Value>,
    key: &'static str,
    wrong: &mut Vec<&'static str>,
) -> Option<T> {
    let value = section.get(key).filter(|value| !value.is_null())?;
    match serde_json::from_value(value.clone()) {
        Ok(value) => Some(value),
        Err(_) => {
            wrong.push(key);
            None
        }
    }
}

impl EditorSettings {
    /// The settings for `cluster`'s editors: its `clusters.<id>.editor` overrides over the user's
    /// values and the defaults (the global value for `None`, or without a settings store, the
    /// shipped defaults).
    ///
    /// The one way the editor reads its settings: views call it where they use a value instead of
    /// keeping a copy, so a reload is never missed.
    pub fn resolve<'a>(cluster: Option<&ClusterId>, cx: &'a App) -> &'a Self {
        cx.try_global::<SettingsStore>()
            .and_then(|store| {
                store.try_get::<Self>(cluster.map(|cluster| SettingsLocation { cluster }))
            })
            .unwrap_or_else(|| {
                static DEFAULT: OnceLock<EditorSettings> = OnceLock::new();
                DEFAULT.get_or_init(EditorSettings::default)
            })
    }

    /// The font as the code editor takes it.
    pub fn font(&self) -> EditorFont {
        EditorFont {
            family: self.font_family.clone(),
            size: self.font_size,
        }
    }
}
