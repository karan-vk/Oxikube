//! `#[gpui::test]`s of [`super::background`]: a focused text field blinks its caret (a redraw
//! every 500 ms) only while its window is active.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, Render, Subscription,
    TestAppContext, VisualTestContext, Window,
};
use oxikube_ui::input::{Input, InputState};
use oxikube_ui::root::Root;

use super::background::follow;

/// A view with one text field and the background rule installed.
struct Field {
    input: Entity<InputState>,
    _background: Subscription,
}

impl Field {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            input: cx.new(|cx| InputState::new(window, cx)),
            _background: follow(window, cx),
        }
    }
}

impl Render for Field {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Input::new(&self.input)
    }
}

/// A window with a focused field, and how many times the field notified since.
struct Rig {
    field: Entity<Field>,
    first: VisualTestContext,
    blinks: Rc<Cell<usize>>,
    _observer: Subscription,
}

impl Rig {
    fn new(cx: &mut TestAppContext) -> Self {
        cx.update(|cx| {
            oxikube_ui::init(cx);
            cx.set_reduce_motion(true);
        });
        let mut field = None;
        let window = cx.add_window(|window, cx| {
            let view = cx.new(|cx| Field::new(window, cx));
            field = Some(view.clone());
            Root::new(view, window, cx)
        });
        let field = field.expect("built");
        let mut first = VisualTestContext::from_window(window.into(), cx);
        first.update(|window, _| window.activate_window());
        first.run_until_parked();
        first.update(|window, cx| {
            let focus = field.read(cx).input.focus_handle(cx);
            window.focus(&focus, cx);
        });
        first.run_until_parked();
        let blinks = Rc::new(Cell::new(0));
        let count = blinks.clone();
        let input = field.read_with(&first, |f, _| f.input.clone());
        let observer =
            first.update(|_, cx| cx.observe(&input, move |_, _| count.set(count.get() + 1)));
        Self {
            field,
            first,
            blinks,
            _observer: observer,
        }
    }

    fn focused(&mut self) -> bool {
        let field = self.field.clone();
        self.first
            .update(|window, cx| field.read(cx).input.focus_handle(cx).is_focused(window))
    }

    /// The blinks (notifies of the field) over `span` of the test clock.
    fn blinks_over(&mut self, span: Duration) -> usize {
        let before = self.blinks.get();
        self.first.executor().advance_clock(span);
        self.first.run_until_parked();
        self.blinks.get() - before
    }

    /// Another window takes the key: the first one is inactive.
    fn deactivate(&mut self, cx: &mut TestAppContext) -> VisualTestContext {
        let other = cx.add_window(|_, _| gpui::EmptyView);
        let mut vcx = VisualTestContext::from_window(other.into(), cx);
        vcx.update(|window, _| window.activate_window());
        vcx.run_until_parked();
        self.first.run_until_parked();
        vcx
    }

    fn reactivate(&mut self) {
        self.first.update(|window, _| window.activate_window());
        self.first.run_until_parked();
    }
}

#[gpui::test]
fn a_focused_field_blinks_while_its_window_is_active(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    assert!(rig.focused());
    // The premise: every blink is a notify, so a redraw.
    assert!(rig.blinks_over(Duration::from_secs(3)) >= 4);
}

#[gpui::test]
fn an_inactive_window_has_nothing_focused_and_nothing_blinking(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    let _other = rig.deactivate(cx);
    assert!(!rig.focused(), "the field was parked");
    assert_eq!(
        rig.blinks_over(Duration::from_secs(10)),
        0,
        "ten seconds in the background: not one redraw for a caret nobody sees"
    );
}

#[gpui::test]
fn the_field_gets_its_focus_and_its_blink_back_with_the_window(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    let _other = rig.deactivate(cx);
    rig.reactivate();
    assert!(rig.focused(), "the same field has the focus again");
    assert!(rig.blinks_over(Duration::from_secs(3)) >= 4);
}

#[gpui::test]
fn a_focus_moved_while_inactive_is_not_taken_back(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    let _other = rig.deactivate(cx);
    // Something else took the focus while the window was in the background.
    let elsewhere = rig.first.update(|_, cx| cx.focus_handle());
    rig.first.update(|window, cx| window.focus(&elsewhere, cx));
    rig.reactivate();
    assert!(!rig.focused(), "the activation left the newer focus alone");
}

#[gpui::test]
fn the_main_window_parks_its_focus_in_the_background(cx: &mut TestAppContext) {
    cx.update(|cx| {
        oxikube_ui::init(cx);
        super::init(cx);
        cx.set_reduce_motion(true);
    });
    let handle = cx.update(super::open_main_window).expect("main window");
    let mut vcx = VisualTestContext::from_window(handle.into(), cx);
    vcx.update(|window, _| window.activate_window());
    vcx.run_until_parked();
    // Something in the window has the focus (the app focuses the catalog's search field).
    let focus = vcx.update(|_, cx| cx.focus_handle());
    vcx.update(|window, cx| window.focus(&focus, cx));
    vcx.run_until_parked();

    let other = cx.add_window(|_, _| gpui::EmptyView);
    let mut other = VisualTestContext::from_window(other.into(), cx);
    other.update(|window, _| window.activate_window());
    other.run_until_parked();
    vcx.run_until_parked();
    assert!(vcx.update(|window, cx| window.focused(cx).is_none()));

    vcx.update(|window, _| window.activate_window());
    vcx.run_until_parked();
    assert!(vcx.update(|window, cx| window.focused(cx)) == Some(focus));
}
