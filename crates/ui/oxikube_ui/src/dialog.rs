//! Dialogs, sheets and toasts, and the window-level helpers that open them.
//!
//! In gpui-component 0.7 the window `Root` owns the dialog, sheet and notification layers and
//! renders them itself, so there is no layer helper to call: build the window root with
//! [`crate::root::new_root`] and open overlays through [`OverlayExt`].

pub use gpui_component::dialog::{
    AlertDialog, Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle,
};
pub use gpui_component::notification::Notification as Toast;
pub use gpui_component::sheet::Sheet;
pub use gpui_component::{Placement, WindowExt as OverlayExt};
