//! [`TestApp`]: the entry point of a `#[gpui::test]`.

use std::time::Duration;

use gpui::{App, Context, KeyBinding, Render, TestAppContext, Window};

use super::TestWindow;

/// A deterministic GPUI app for one `#[gpui::test]`.
///
/// It wraps the [`TestAppContext`] the `#[gpui::test]` macro hands to the test function. Time
/// only moves when the test says so ([`TestApp::advance_clock`]), tasks only run when the test
/// lets them ([`TestApp::run_until_parked`], which every helper here also does), and nothing
/// starts an OS thread. See `docs/testing-gpui.md` for the rules and a worked example per helper.
///
/// ```ignore
/// #[gpui::test]
/// fn pressing_j_pings(cx: &mut TestAppContext) {
///     let mut app = TestApp::new(cx);
///     app.bind_keys([KeyBinding::new("j", Ping, None)]);
///     let mut window = app.open_window(|_, _| MyView::default());
///     window.simulate_keystrokes("j");
///     assert_eq!(window.read_root(|view, _| view.pings), 1);
/// }
/// ```
pub struct TestApp {
    cx: TestAppContext,
}

impl TestApp {
    /// Wraps the context `#[gpui::test]` passed to the test.
    pub fn new(cx: &TestAppContext) -> Self {
        Self { cx: cx.clone() }
    }

    /// The underlying context, for anything this wrapper does not cover.
    pub fn cx(&mut self) -> &mut TestAppContext {
        &mut self.cx
    }

    /// Runs `f` with the [`App`], then runs every task it made runnable.
    pub fn update<R>(&mut self, f: impl FnOnce(&mut App) -> R) -> R {
        let result = self.cx.update(f);
        self.cx.run_until_parked();
        result
    }

    /// Runs `f` with a shared [`App`].
    pub fn read<R>(&self, f: impl FnOnce(&App) -> R) -> R {
        self.cx.read(f)
    }

    /// Registers key bindings (what the app's `keymap.json` layers do in production), so that
    /// [`TestWindow::simulate_keystrokes`] reaches the actions.
    pub fn bind_keys(&mut self, bindings: impl IntoIterator<Item = KeyBinding>) {
        self.update(|cx| cx.bind_keys(bindings));
    }

    /// Opens a window whose root is the view `build` returns, and activates it (focus events only
    /// reach the active window).
    ///
    /// For an app-level window (a `Workspace` inside a `Root`) see
    /// `oxikube_workspace::test_support::open_workspace`, which cannot live here because the
    /// testkit sits below the UI layers.
    pub fn open_window<V: Render + 'static>(
        &mut self,
        build: impl FnOnce(&mut Window, &mut Context<V>) -> V,
    ) -> TestWindow<V> {
        let handle = self.cx.add_window(build);
        TestWindow::new(handle, &self.cx)
    }

    /// Lets every runnable task run until there is nothing left to do (the test equivalent of
    /// "wait for the UI to settle"; it never waits for real time).
    pub fn run_until_parked(&self) {
        self.cx.run_until_parked();
    }

    /// Moves the test clock forward by `duration`, firing the timers and debounces that fall due,
    /// and runs what they wake. This is the replacement for `thread::sleep`.
    pub fn advance_clock(&self, duration: Duration) {
        self.cx.executor().advance_clock(duration);
        self.cx.run_until_parked();
    }
}
