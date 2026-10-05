//! The session settings: root-level keys of `settings.json`.

use gpui::App;
use oxikube_settings::Settings;
use oxikube_ui::UiScale;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How the `reduce_motion` setting is decided.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReduceMotionSetting {
    /// Follow the operating system's reduce-motion preference.
    #[default]
    System,
    /// Always reduce motion, whatever the OS says.
    On,
    /// Never reduce motion, whatever the OS says.
    Off,
}

/// What one settings layer says about the session.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct SessionSettingsContent {
    /// UI zoom factor: 1.0 is 100 %. Values outside 0.5 to 3.0 are clamped. `cmd +`, `cmd -` and
    /// `cmd 0` (`ctrl` elsewhere) change it and write it back here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_scale: Option<f32>,
    /// `system` follows the OS reduce-motion preference, `on` and `off` override it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reduce_motion: Option<ReduceMotionSetting>,
    /// Ask before quitting (or closing the last window on Linux and Windows) while exec sessions,
    /// port-forwards or applies are running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_quit: Option<bool>,
}

/// The resolved session settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SessionSettings {
    /// UI zoom, already clamped.
    pub ui_scale: UiScale,
    /// How to decide reduce-motion.
    pub reduce_motion: ReduceMotionSetting,
    /// Whether a quit with running operations asks first.
    pub confirm_quit: bool,
}

impl Settings for SessionSettings {
    const KEY: Option<&'static str> = None;
    type Content = SessionSettingsContent;

    fn from_content(content: SessionSettingsContent) -> Self {
        Self {
            ui_scale: UiScale::new(content.ui_scale.unwrap_or(1.0)),
            reduce_motion: content.reduce_motion.unwrap_or_default(),
            confirm_quit: content.confirm_quit.unwrap_or(true),
        }
    }
}

oxikube_settings::register_settings!(SessionSettings);

/// Applies the zoom and reduce-motion the settings ask for now, and again on every change of
/// them (a hot reload of `settings.json`). Without a settings store it keeps the defaults.
pub(super) fn apply_and_observe(cx: &mut App) {
    apply(cx);
    SessionSettings::observe(cx, |cx| {
        // A zoom of ours is being written: the file has not caught up with the screen yet, and
        // applying it now would flash the old size. `zoom::finish_write` re-syncs afterwards.
        if !super::zoom::write_pending(cx) {
            apply(cx);
        }
    })
    .detach();
}

/// Brings the UI zoom and the reduce-motion flag in line with the settings.
pub(super) fn apply(cx: &mut App) {
    if let Some(settings) = SessionSettings::try_get(cx).copied()
        && UiScale::get(cx) != settings.ui_scale
    {
        oxikube_ui::set_ui_scale(cx, settings.ui_scale);
    }
    super::motion::apply(cx);
}
