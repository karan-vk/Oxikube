//! The window title bar: our own strip under a transparent native title bar (macOS, Windows) or
//! between client-side decorations (Linux).
//!
//! [`TitleBar`] owns dragging, double-click zoom, the Linux/Windows min/max/close controls and the
//! left inset that clears the macOS traffic lights. The window code (E05-S03) builds its
//! `WindowOptions` from [`TitleBar::window_options`] and adds its own title and app id.

pub use gpui_component::{TITLE_BAR_HEIGHT, TitleBar};
