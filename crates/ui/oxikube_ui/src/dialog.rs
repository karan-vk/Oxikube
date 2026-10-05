//! Dialogs, sheets and toasts, and the window-level helpers that open them.
//!
//! In gpui-component 0.7 the window `Root` owns the dialog, sheet and notification layers and
//! renders them itself, so there is no layer helper to call: build the window root with
//! [`crate::root::new_root`] and open overlays through [`OverlayExt`].

pub use gpui_component::dialog::{
    AlertDialog, Cancel, Confirm, Dialog, DialogContent, DialogDescription, DialogFooter,
    DialogHeader, DialogTitle,
};
pub use gpui_component::notification::Notification as Toast;
pub use gpui_component::sheet::Sheet;
pub use gpui_component::{Placement, WindowExt as OverlayExt};

// `Cancel` and `Confirm` are the library's Escape / Enter actions: components that handle them
// (lists, selects, popup menus, inputs) consume the key before an enclosing modal layer sees it,
// so a modal built on them closes only when nothing inside wants the key.
