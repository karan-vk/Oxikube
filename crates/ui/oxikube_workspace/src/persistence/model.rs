//! The on-disk shape of a saved window layout, and its versioning.
//!
//! A layout is gpui-component's [`DockAreaState`] (the split and tab tree of every dock region,
//! with dock sizes and open flags) plus the things the dock area does not know: the open item
//! descriptors (each item tab's `PanelInfo` carries `{ "kind", "state" }`, see
//! [`ITEM_PANEL_NAME`](crate::item::ITEM_PANEL_NAME)), which pane was active, and where the
//! window was. It is wrapped in a versioned envelope so a future change of shape is detected
//! instead of mis-read.
//!
//! Dock sizes are stored **unscaled** (zoom-independent, see `oxikube_ui::size`). Nothing secret
//! is ever stored here: items and panels must not put secrets in their serialised state.

use oxikube_ui::dock::DockAreaState;
use serde::{Deserialize, Serialize};

/// The envelope version this build writes and reads.
pub const LAYOUT_SCHEMA_VERSION: u32 = 1;

/// The [`StatePort`](oxikube_ports::StatePort) table layouts are stored in, one row per window.
pub const LAYOUT_TABLE: &str = "workspace_layout";

/// The window id of the main window, the only window until multi-window support exists.
pub const MAIN_WINDOW_ID: &str = "main";

/// How the window was shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowMode {
    /// A normal window at the saved bounds.
    Windowed,
    /// Maximised; the bounds are what the window returns to when un-maximised.
    Maximized,
    /// Full screen; the bounds are what the window returns to when leaving full screen.
    Fullscreen,
}

/// Where the window was, in logical pixels of the global screen space.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedWindow {
    /// Windowed, maximised or full screen.
    pub mode: WindowMode,
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

/// A saved window layout: the envelope written to the state store.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedWorkspace {
    /// [`LAYOUT_SCHEMA_VERSION`] of the build that wrote it.
    pub version: u32,
    /// Where the window was, when known.
    #[serde(default)]
    pub window: Option<SerializedWindow>,
    /// The active centre pane, as its index among the panes that hold a restorable item, in
    /// layout order.
    #[serde(default)]
    pub active_pane: Option<usize>,
    /// The dock area: centre splits and tab groups with the item descriptors, and the docks.
    pub dock_area: DockAreaState,
}

/// Why a stored layout cannot be used.
#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    /// Written by a newer build whose shape this one does not know.
    #[error("layout was written by a newer build (schema {found}, this build reads {supported})")]
    Newer {
        /// The stored schema version.
        found: u64,
        /// [`LAYOUT_SCHEMA_VERSION`].
        supported: u32,
    },
    /// Not a layout at all: missing version, wrong shape, or an unknown old version.
    #[error("stored layout is unreadable: {0}")]
    Malformed(String),
}

impl SerializedWorkspace {
    /// The JSON the state store keeps.
    pub fn to_json(&self) -> serde_json::Value {
        // A struct of plain data: serialisation cannot fail.
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    /// Reads a stored layout, checking the version first so that a newer shape is reported as
    /// such rather than as a parse error.
    ///
    /// # Errors
    ///
    /// [`LayoutError::Newer`] for a newer schema, [`LayoutError::Malformed`] otherwise.
    pub fn from_json(value: serde_json::Value) -> Result<Self, LayoutError> {
        let found = value
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| LayoutError::Malformed("no schema version".into()))?;
        if found > u64::from(LAYOUT_SCHEMA_VERSION) {
            return Err(LayoutError::Newer {
                found,
                supported: LAYOUT_SCHEMA_VERSION,
            });
        }
        // Older schemas are migrated here, oldest first, once any exist. Version 1 is the first.
        if found != u64::from(LAYOUT_SCHEMA_VERSION) {
            return Err(LayoutError::Malformed(format!("unknown schema {found}")));
        }
        serde_json::from_value(value).map_err(|e| LayoutError::Malformed(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use oxikube_ui::dock::{PanelInfo, PanelState};
    use serde_json::json;

    use super::*;

    fn layout() -> SerializedWorkspace {
        let mut tabs = PanelState::new("TabPanel");
        tabs.info = PanelInfo::tabs(0);
        SerializedWorkspace {
            version: LAYOUT_SCHEMA_VERSION,
            window: Some(SerializedWindow {
                mode: WindowMode::Maximized,
                x: 10.0,
                y: 20.0,
                width: 1280.0,
                height: 800.0,
            }),
            active_pane: Some(1),
            dock_area: DockAreaState {
                center: tabs,
                ..Default::default()
            },
        }
    }

    #[test]
    fn round_trips_through_json() {
        let saved = layout();
        assert_eq!(
            SerializedWorkspace::from_json(saved.to_json()).unwrap(),
            saved
        );
    }

    #[test]
    fn optional_fields_may_be_absent() {
        let mut value = layout().to_json();
        let obj = value.as_object_mut().unwrap();
        obj.remove("window");
        obj.remove("active_pane");
        let read = SerializedWorkspace::from_json(value).unwrap();
        assert_eq!((read.window, read.active_pane), (None, None));
    }

    #[test]
    fn a_newer_schema_is_reported_as_such_even_when_the_shape_changed() {
        let err =
            SerializedWorkspace::from_json(json!({ "version": 99, "panes": [] })).unwrap_err();
        assert!(matches!(err, LayoutError::Newer { found: 99, .. }), "{err}");
    }

    #[test]
    fn garbage_is_malformed() {
        for value in [
            json!(null),
            json!("text"),
            json!({}),
            json!({ "version": "one" }),
            json!({ "version": 0 }),
            json!({ "version": 1, "dock_area": 3 }),
        ] {
            let err = SerializedWorkspace::from_json(value.clone()).unwrap_err();
            assert!(matches!(err, LayoutError::Malformed(_)), "{value}: {err}");
        }
    }
}
