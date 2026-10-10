//! Tooltips that name a key (E11-S10): the binding comes from the installed keymap, in the
//! context the tooltip is given, and follows a rebind.

use std::time::Duration;

use gpui::{
    Context, InteractiveElement as _, IntoElement, KeyBinding, Modifiers, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, TestAppContext, VisualTestContext,
    Window, actions, div, px,
};
use oxikube_ui::kbd::{Kbd, binding_keystroke};
use oxikube_ui::tooltip::tooltip_for_action;

actions!(tooltip_test, [Wrap, Unbound]);

struct Hover;

impl Render for Hover {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            div()
                .id("subject")
                .debug_selector(|| "subject".into())
                .size(px(40.))
                .tooltip(tooltip_for_action("Wrap lines", &Wrap, Some("LogView"))),
        )
    }
}

fn open(cx: &mut TestAppContext) -> &mut VisualTestContext {
    cx.update(oxikube_ui::init);
    let (_view, cx) = cx.add_window_view(|_, _| Hover);
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx
}

fn hint(cx: &mut VisualTestContext, action: &dyn gpui::Action, context: Option<&str>) -> String {
    cx.update(|window, _| {
        binding_keystroke(action, context, window)
            .map(|stroke| Kbd::format(&stroke))
            .unwrap_or_default()
    })
}

#[gpui::test]
fn the_hint_is_the_strongest_binding_in_the_given_context(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.bind_keys([
            KeyBinding::new("w", Wrap, Some("LogView && !Editing")),
            KeyBinding::new("ctrl-w", Wrap, Some("ResourceTable")),
        ])
    });
    let cx = open(cx);
    let expected = |keys: &str| Kbd::format(&gpui::Keystroke::parse(keys).expect("a keystroke"));
    assert_eq!(hint(cx, &Wrap, Some("LogView")), expected("w"));
    assert_eq!(hint(cx, &Wrap, Some("ResourceTable")), expected("ctrl-w"));
    assert_eq!(hint(cx, &Wrap, Some("Terminal")), "", "no binding there");
    assert_eq!(hint(cx, &Unbound, Some("LogView")), "", "nothing bound");
    assert_eq!(hint(cx, &Wrap, None), "", "both bindings have a context");
}

#[gpui::test]
fn a_rebind_shows_in_the_next_hint(cx: &mut TestAppContext) {
    cx.update(|cx| cx.bind_keys([KeyBinding::new("w", Wrap, Some("LogView"))]));
    let cx = open(cx);
    assert_eq!(
        hint(cx, &Wrap, Some("LogView")),
        Kbd::format(&gpui::Keystroke::parse("w").unwrap())
    );
    // The keymap layers rebuild their bindings on a reload; a later binding wins.
    cx.update(|_, cx| cx.bind_keys([KeyBinding::new("x", Wrap, Some("LogView"))]));
    assert_eq!(
        hint(cx, &Wrap, Some("LogView")),
        Kbd::format(&gpui::Keystroke::parse("x").unwrap())
    );
}

fn hover(cx: &mut VisualTestContext) {
    let subject = cx.debug_bounds("subject").expect("the element is drawn");
    cx.simulate_mouse_move(subject.center(), None, Modifiers::default());
    for _ in 0..2 {
        cx.executor().advance_clock(Duration::from_secs(2));
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
}

#[gpui::test]
fn hovering_a_bound_action_shows_the_title_and_the_key(cx: &mut TestAppContext) {
    cx.update(|cx| cx.bind_keys([KeyBinding::new("w", Wrap, Some("LogView"))]));
    let cx = open(cx);
    assert!(cx.debug_bounds("action-tooltip-title").is_none());
    hover(cx);
    assert!(
        cx.debug_bounds("action-tooltip-title").is_some(),
        "the tooltip opened"
    );
    assert!(
        cx.debug_bounds("action-tooltip-key").is_some(),
        "the binding is shown next to the title"
    );
}

#[gpui::test]
fn an_action_with_no_binding_shows_the_title_alone(cx: &mut TestAppContext) {
    // `Wrap` has no binding in this context.
    cx.update(|cx| cx.bind_keys([KeyBinding::new("w", Wrap, Some("ResourceTable"))]));
    let cx = open(cx);
    hover(cx);
    assert!(cx.debug_bounds("action-tooltip-title").is_some());
    assert!(cx.debug_bounds("action-tooltip-key").is_none());
}
