//! The page's keys.

use gpui::TestAppContext;
use oxikube_app::rbac_subjects::MatrixColumn;
use oxikube_domain::kinds::Verb;

use super::Fx;
use crate::subjects::Page;

#[gpui::test]
fn the_arrows_move_the_selection_and_enter_opens_the_subject(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.users();
    fx.keys("down");
    fx.vcx.update(|_, cx| {
        assert!(view.read(cx).list().read(cx, |d| d.selected().is_some()));
    });
    fx.keys("j");
    let second = fx
        .vcx
        .update(|_, cx| view.read(cx).list().read(cx, |d| d.selected().cloned()));
    fx.keys("k");
    let first = fx
        .vcx
        .update(|_, cx| view.read(cx).list().read(cx, |d| d.selected().cloned()));
    assert_ne!(first, second);
    fx.keys("enter");
    fx.vcx
        .update(|_, cx| assert_eq!(view.read(cx).page(), Page::Subject));
    fx.keys("escape");
    fx.vcx
        .update(|_, cx| assert_eq!(view.read(cx).page(), Page::Users));
}

#[gpui::test]
fn h_and_l_pick_the_verb_column_and_s_cycles_the_scope(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.subject("fx-viewer");
    fx.keys("down");
    fx.keys("l");
    fx.vcx.update(|_, cx| {
        assert_eq!(view.read(cx).column(), MatrixColumn::Verb(Verb::List));
    });
    fx.keys("h h");
    fx.vcx.update(|_, cx| {
        assert_eq!(view.read(cx).column(), MatrixColumn::Verb(Verb::Get));
    });
    let before = fx.vcx.update(|_, cx| view.read(cx).scope().clone());
    fx.keys("s");
    let after = fx.vcx.update(|_, cx| view.read(cx).scope().clone());
    assert_ne!(before, after);
}

#[gpui::test]
fn the_slash_goes_to_the_filter_and_escape_gives_the_rows_back(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.users();
    fx.keys("/");
    fx.keys("f x - e d");
    fx.vcx.update(|_, cx| {
        assert_eq!(
            view.read(cx).list().read(cx, |d| d.shown_names()),
            ["fx-editor"]
        );
    });
    fx.keys("escape");
    // Bare keys move the cursor again.
    fx.keys("j");
    fx.vcx.update(|_, cx| {
        assert!(view.read(cx).list().read(cx, |d| d.selected().is_some()));
    });
}
