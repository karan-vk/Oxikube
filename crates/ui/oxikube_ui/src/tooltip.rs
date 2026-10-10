//! Tooltips: text (or an element) shown after hovering an element for a moment.
//!
//! [`tooltip_for_action`] is the one way a tooltip names a key (E11-S10): the title plus the
//! binding the keymap has for an action *now*, so a `keymap.json` rebind shows up in the next
//! tooltip. A [`Button`](crate::button::Button) already has the same behaviour built in
//! (`Button::tooltip_with_action(title, &action, Some("Context"))`); this helper is for the other
//! elements (`div().id(..).tooltip(tooltip_for_action(..))`).

use gpui::{
    Action, AnyView, App, InteractiveElement as _, ParentElement as _, SharedString, Styled as _,
    Window, div,
};
pub use gpui_component::tooltip::Tooltip;

use crate::kbd::{Kbd, binding_keystroke};
use crate::layout::h_flex;
use crate::size::u;
use crate::tokens::ActiveTokens as _;

/// A tooltip builder for `.tooltip(..)` on a stateful element: `title`, and the key that runs
/// `action` when the key context `context` is in force (`None`: a binding without a context
/// predicate). The binding is looked up each time the tooltip opens; with no binding the tooltip
/// is the title alone.
pub fn tooltip_for_action(
    title: impl Into<SharedString>,
    action: &dyn Action,
    context: Option<&str>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let title: SharedString = title.into();
    let action = action.boxed_clone();
    let context: Option<SharedString> = context.map(|context| context.to_owned().into());
    move |window, cx| {
        let hint = binding_keystroke(action.as_ref(), context.as_deref(), window).map(Kbd::new);
        let title = title.clone();
        Tooltip::element(move |_, cx| {
            let colors = cx.colors();
            let key = hint.clone().map(|kbd| {
                div()
                    .debug_selector(|| "action-tooltip-key".to_owned())
                    .flex_shrink_0()
                    .text_size(u(cx.tokens().font.small))
                    .text_color(colors.text_muted)
                    .child(kbd.appearance(false))
            });
            h_flex()
                .gap(u(cx.tokens().spacing.lg))
                .child(
                    div()
                        .debug_selector(|| "action-tooltip-title".to_owned())
                        .child(title.clone()),
                )
                .children(key)
        })
        .build(window, cx)
    }
}
