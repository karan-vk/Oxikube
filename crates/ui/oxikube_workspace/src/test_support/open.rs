//! [`open_workspace`]: the workspace window every `#[gpui::test]` of a workspace consumer starts
//! from.

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use oxikube_ui::root::Root;

use super::register_test_item;
use crate::Workspace;

/// Opens an active window whose root hosts a fresh [`Workspace`], and returns the workspace with
/// the window's [`VisualTestContext`] (which has `simulate_keystrokes`, `dispatch_action`,
/// `run_until_parked`, ...; `oxikube_testkit::gpui_test::TestWindow` derefs to the same type).
///
/// Sets the app up the way the binary does: the `oxikube_ui` globals, the workspace, modal and
/// toast actions and key bindings, and the [`TestItem`](super::TestItem) builder so closed test
/// items can be reopened. Reduce-motion is on, so no fade keeps asking for frames while the test
/// clock stands still.
///
/// ```ignore
/// #[gpui::test]
/// fn opens_an_item(cx: &mut TestAppContext) {
///     let (workspace, mut vcx) = open_workspace(cx);
///     vcx.update(|window, cx| {
///         let item = TestItem::build("Pods", cx);
///         workspace.update(cx, |ws, cx| ws.open_item(item, window, cx));
///     });
///     vcx.run_until_parked();
/// }
/// ```
///
/// Calling it again in the same test opens a second window in the same app.
pub fn open_workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, VisualTestContext) {
    cx.update(|cx| {
        oxikube_ui::init(cx);
        crate::actions::register(cx);
        crate::modal::register(cx);
        crate::toast::register(cx);
        cx.set_reduce_motion(true);
        register_test_item(cx);
    });
    let mut workspace = None;
    let window = cx.add_window(|window, cx| {
        let entity = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    // Focus events (which make a pane active) are only delivered to the active window.
    vcx.update(|window, _| window.activate_window());
    vcx.run_until_parked();
    (workspace.expect("the window was built"), vcx)
}
