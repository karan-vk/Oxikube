//! `oxikube_workspace` — layer: `ui`.
//!
//! Window shell: Item/Panel/Pane/Dock model, tabs, status bar, modal/toast layers, layout persistence, cluster tabs, sidebar, notifications panel.
//!
//! Module map:
//! - [`window`]: the main window (E05-S03): platform window options (native title bar on macOS,
//!   client-side decorations and the Wayland app id on Linux), the `Root` that hosts the overlay
//!   layers, the title bar, and the application menu.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod window;
