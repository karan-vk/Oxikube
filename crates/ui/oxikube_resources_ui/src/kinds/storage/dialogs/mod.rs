//! The two storage dialogs (E12-S15): [`ResizeDialog`] and [`DefaultClassDialog`], modals of the
//! cluster tab's workspace. Both read what they show off the UI thread, send one command through
//! the bus (the guard's simple confirmation is their button) and close with a toast.

mod default_class;
mod resize;
mod resize_view;

pub use default_class::{ClassLoaded, ClassStage, DefaultClassDialog};
pub use resize::{Loaded, ResizeDialog, ResizeStage};
