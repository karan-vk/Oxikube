//! [`TestWindow`]: a test window with the verbs a UI test needs.

use std::{
    ops::{Deref, DerefMut},
    time::Duration,
};

use gpui::{
    Action, App, Bounds, Context, Entity, Pixels, Render, TestAppContext, VisualTestContext,
    Window, WindowHandle,
};

/// A window opened by [`TestApp::open_window`](super::TestApp::open_window) (or wrapped from any
/// handle with [`TestWindow::new`]).
///
/// It dereferences to gpui's [`VisualTestContext`], so everything gpui offers (mouse events,
/// `draw`, `simulate_resize`, ...) is available too. The methods here are the ones every UI story
/// needs; each one runs the tasks it made runnable before it returns, so an assertion that
/// follows sees the settled state.
pub struct TestWindow<V> {
    handle: WindowHandle<V>,
    vcx: VisualTestContext,
}

impl<V: Render + 'static> TestWindow<V> {
    /// Wraps `handle`, activates the window and lets it settle.
    pub fn new(handle: WindowHandle<V>, cx: &TestAppContext) -> Self {
        let mut vcx = VisualTestContext::from_window(handle.into(), cx);
        // Focus events (which make a pane or an input the focused one) are only delivered to the
        // active window.
        vcx.update(|window, _| window.activate_window());
        vcx.run_until_parked();
        Self { handle, vcx }
    }

    /// The typed window handle.
    pub fn handle(&self) -> WindowHandle<V> {
        self.handle
    }

    /// The root view entity.
    pub fn root(&mut self) -> Entity<V> {
        self.handle
            .entity(&self.vcx)
            .expect("the window is still open")
    }

    /// Updates the root view, then settles.
    pub fn update_root<R>(
        &mut self,
        f: impl FnOnce(&mut V, &mut Window, &mut Context<V>) -> R,
    ) -> R {
        let result = self
            .handle
            .update(&mut self.vcx, f)
            .expect("the window is still open");
        self.vcx.run_until_parked();
        result
    }

    /// Reads the root view.
    pub fn read_root<R>(&mut self, f: impl FnOnce(&V, &App) -> R) -> R {
        let root = self.root();
        self.vcx.read(|cx| f(root.read(cx), cx))
    }

    /// Types a space-separated list of keystrokes (`"cmd-k escape"`) into the window, through the
    /// bound keymap, exactly like a user. Bind the keys first with
    /// [`TestApp::bind_keys`](super::TestApp::bind_keys).
    pub fn simulate_keystrokes(&mut self, keystrokes: &str) {
        self.vcx.simulate_keystrokes(keystrokes);
    }

    /// Types `text` as input (one keystroke per character).
    pub fn simulate_input(&mut self, text: &str) {
        self.vcx.simulate_input(text);
    }

    /// Dispatches `action` to the focused element (what the command palette and menus do).
    pub fn dispatch_action<A: Action>(&mut self, action: A) {
        self.vcx.dispatch_action(action);
    }

    /// Moves the test clock forward by `duration` and runs what falls due.
    pub fn advance_clock(&self, duration: Duration) {
        self.vcx.executor().advance_clock(duration);
        self.vcx.run_until_parked();
    }

    /// Draws one frame now (layout, prepaint, paint), so [`TestWindow::bounds`] and hit testing
    /// see the latest state.
    pub fn draw_frame(&mut self) {
        self.vcx.update(|window, cx| window.draw(cx).clear(cx));
        self.vcx.run_until_parked();
    }

    /// The bounds of the element tagged `.debug_selector(|| "name".into())` in the last frame.
    pub fn bounds(&mut self, selector: &'static str) -> Option<Bounds<Pixels>> {
        self.vcx.debug_bounds(selector)
    }
}

impl<V> Deref for TestWindow<V> {
    type Target = VisualTestContext;

    fn deref(&self) -> &VisualTestContext {
        &self.vcx
    }
}

impl<V> DerefMut for TestWindow<V> {
    fn deref_mut(&mut self) -> &mut VisualTestContext {
        &mut self.vcx
    }
}
