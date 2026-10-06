//! The GPUI test harness (E05-S11): helpers every `#[gpui::test]` of the project starts from.
//!
//! Feature `gpui-test`. Everything here is deterministic: no OS threads, no wall-clock waits, no
//! GPU, so it runs the same on libtest worker threads on macOS and Linux CI.
//!
//! | Need | Helper |
//! |---|---|
//! | an app and its window | [`TestApp::new`], [`TestApp::open_window`] |
//! | a workspace window | `oxikube_workspace::test_support::open_workspace` (above this layer) |
//! | key bindings, keystrokes | [`TestApp::bind_keys`], [`TestWindow::simulate_keystrokes`] |
//! | an action | [`TestWindow::dispatch_action`] |
//! | time | [`TestApp::advance_clock`], [`TestWindow::advance_clock`] |
//! | tasks | [`TestApp::run_until_parked`] |
//! | fake ports and `AppState` | [`crate::TestPorts`], `oxikube::app_state::AppState::test_with` |
//! | a screenshot compared to a golden | `ScreenshotApp` (feature `gpui-screenshot`) |
//!
//! The rules, the "don't" list and a worked example per helper are in `docs/testing-gpui.md`.

mod app;
#[cfg(feature = "gpui-screenshot")]
mod shot;
mod window;

pub use app::TestApp;
#[cfg(feature = "gpui-screenshot")]
pub use shot::{GoldenCase, ScreenshotApp, run_golden_cases};
pub use window::TestWindow;
