//! [`ConfirmDialog`] (E19-S06): the two tiers, the keyboard, every way out, and the bulk list.

use std::{cell::RefCell, rc::Rc};

use gpui::{Modifiers, VisualTestContext, point, px};
use oxikube_domain::audit::Initiator;
use oxikube_domain::safety::Risk;

use super::*;
use crate::modal::{ConfirmDialog, ConfirmItem, ConfirmPrompt, ModalLayer, PreviewNote};

/// What the dialog told its opener.
#[derive(Default)]
struct Answers {
    confirmed: Vec<Option<String>>,
    declined: usize,
}

fn open_dialog(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    prompt: ConfirmPrompt,
) -> (Entity<ConfirmDialog>, Rc<RefCell<Answers>>) {
    let answers = Rc::new(RefCell::new(Answers::default()));
    let (on_confirm, on_decline) = (answers.clone(), answers.clone());
    let dialog = vcx.update(|window, cx| {
        let dialog = cx.new(|cx| {
            ConfirmDialog::new(
                prompt,
                move |typed, _, _| on_confirm.borrow_mut().confirmed.push(typed),
                move |_| on_decline.borrow_mut().declined += 1,
                window,
                cx,
            )
        });
        ws.update(cx, |ws, cx| ws.show_modal(dialog.clone(), window, cx));
        dialog.update(cx, |dialog, cx| dialog.focus_initial(window, cx));
        dialog
    });
    vcx.run_until_parked();
    (dialog, answers)
}

fn layer(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> Entity<ModalLayer> {
    vcx.update(|_, cx| ws.read(cx).modal_layer().clone())
}

fn is_open(layer: &Entity<ModalLayer>, vcx: &mut VisualTestContext) -> bool {
    vcx.update(|_, cx| layer.read(cx).has_active_modal())
}

fn can_confirm(dialog: &Entity<ConfirmDialog>, vcx: &mut VisualTestContext) -> bool {
    vcx.update(|_, cx| dialog.read(cx).can_confirm(cx))
}

fn type_text(dialog: &Entity<ConfirmDialog>, vcx: &mut VisualTestContext, text: &str) {
    vcx.update(|window, cx| dialog.update(cx, |d, cx| d.type_text(text, window, cx)));
    vcx.run_until_parked();
}

fn typed_prompt() -> ConfirmPrompt {
    ConfirmPrompt::new(
        "Delete the namespace?",
        "Deletes everything in kube-system.",
    )
    .with_confirm_label("Delete")
    .with_destructive(true)
    .with_typed_name("kube-system")
}

#[gpui::test]
fn type_the_name_disables_confirm_until_the_name_matches_exactly(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let (dialog, answers) = open_dialog(&ws, &mut vcx, typed_prompt());
    assert!(!can_confirm(&dialog, &mut vcx), "empty");
    for wrong in ["kube", "Kube-System", " kube-system", "kube-system "] {
        type_text(&dialog, &mut vcx, wrong);
        assert!(!can_confirm(&dialog, &mut vcx), "{wrong:?} is not the name");
    }
    type_text(&dialog, &mut vcx, "kube-system");
    assert!(can_confirm(&dialog, &mut vcx));
    assert!(
        answers.borrow().confirmed.is_empty(),
        "typing confirms nothing"
    );
}

#[gpui::test]
fn enter_is_ignored_while_the_name_is_wrong_and_confirms_once_it_matches(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);
    let (dialog, answers) = open_dialog(&ws, &mut vcx, typed_prompt());
    // The field has focus, not the Confirm button.
    vcx.simulate_keystrokes("enter");
    type_text(&dialog, &mut vcx, "kube-syste");
    vcx.simulate_keystrokes("enter");
    assert!(answers.borrow().confirmed.is_empty());
    assert_eq!(answers.borrow().declined, 0);
    assert!(is_open(&layer, &mut vcx), "the dialog stays");

    type_text(&dialog, &mut vcx, "kube-system");
    vcx.simulate_keystrokes("enter");
    assert_eq!(
        answers.borrow().confirmed,
        vec![Some("kube-system".to_owned())],
        "the answer carries what was typed"
    );
    assert!(!is_open(&layer, &mut vcx));
    drop(dialog);
    vcx.run_until_parked();
    assert_eq!(
        answers.borrow().declined,
        0,
        "a confirmed dialog never declines"
    );
}

#[gpui::test]
fn the_name_field_is_focused_and_labelled_with_the_expected_name(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let (_dialog, _answers) = open_dialog(&ws, &mut vcx, typed_prompt());
    assert!(bounds(&mut vcx, "dialog-type-input").is_some());
    assert!(bounds(&mut vcx, "dialog-type-prompt").is_some());
    // Typing goes straight into the field: no click needed.
    vcx.simulate_input("kube-system");
    vcx.run_until_parked();
    assert!(can_confirm(&_dialog, &mut vcx));
}

#[gpui::test]
fn escape_cancels_both_tiers_and_declines_exactly_once(cx: &mut TestAppContext) {
    for prompt in [
        typed_prompt(),
        ConfirmPrompt::new("Restart?", "Restart web."),
    ] {
        let (ws, mut vcx) = workspace(cx);
        let layer = layer(&ws, &mut vcx);
        let (dialog, answers) = open_dialog(&ws, &mut vcx, prompt);
        vcx.simulate_keystrokes("escape");
        vcx.run_until_parked();
        assert!(!is_open(&layer, &mut vcx));
        drop(dialog);
        vcx.run_until_parked();
        let answers = answers.borrow();
        assert_eq!(answers.declined, 1);
        assert!(answers.confirmed.is_empty());
    }
}

#[gpui::test]
fn the_simple_tier_confirms_on_enter_without_a_typed_name(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let (dialog, answers) = open_dialog(
        &ws,
        &mut vcx,
        ConfirmPrompt::new("Restart?", "Restart web."),
    );
    assert!(can_confirm(&dialog, &mut vcx));
    assert!(bounds(&mut vcx, "dialog-type-input").is_none());
    vcx.simulate_keystrokes("enter");
    assert_eq!(answers.borrow().confirmed, vec![None]);
}

#[gpui::test]
fn a_click_on_the_scrim_a_replacing_modal_and_a_closing_window_each_decline(
    cx: &mut TestAppContext,
) {
    // The scrim.
    let (ws, mut vcx) = workspace(cx);
    let (_dialog, answers) = open_dialog(&ws, &mut vcx, typed_prompt());
    vcx.simulate_click(point(px(2.), px(2.)), Modifiers::none());
    vcx.run_until_parked();
    drop(_dialog);
    vcx.run_until_parked();
    assert_eq!(answers.borrow().declined, 1, "scrim");

    // Another modal replacing it.
    let (ws, mut vcx) = workspace(cx);
    let (dialog, answers) = open_dialog(&ws, &mut vcx, typed_prompt());
    drop(dialog);
    vcx.update(|window, cx| {
        let other = cx.new(|cx| crate::test_support::TestModal::new("other", cx));
        ws.update(cx, |ws, cx| ws.show_modal(other, window, cx));
    });
    vcx.run_until_parked();
    assert_eq!(answers.borrow().declined, 1, "replaced");

    // The window closing.
    let (ws, mut vcx) = workspace(cx);
    let (dialog, answers) = open_dialog(&ws, &mut vcx, typed_prompt());
    drop(dialog);
    drop(ws);
    vcx.update(|window, _| window.remove_window());
    vcx.run_until_parked();
    assert_eq!(answers.borrow().declined, 1, "window closed");
    assert!(answers.borrow().confirmed.is_empty());
}

fn items(count: usize) -> Vec<ConfirmItem> {
    (0..count)
        .map(|ix| {
            let risk = if ix == 0 {
                Risk::Irreversible
            } else {
                Risk::Medium
            };
            ConfirmItem::new(format!("Pod default/web-{ix}"), risk)
        })
        .collect()
}

#[gpui::test]
fn a_bulk_list_of_500_builds_only_the_rows_on_screen(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let prompt = ConfirmPrompt::new("Delete 500 objects?", "Deletes 500 pods.")
        .with_typed_name("kind-oxikube")
        .with_items(items(500));
    let (_dialog, _answers) = open_dialog(&ws, &mut vcx, prompt);
    assert!(bounds(&mut vcx, "confirm-count").is_some());
    let drawn = (0..500)
        .filter(|ix| bounds_named(&mut vcx, format!("confirm-item-{ix}")).is_some())
        .count();
    assert!(drawn > 0, "the first rows are drawn");
    assert!(drawn <= 12, "only the visible rows are built, got {drawn}");
    assert!(bounds_named(&mut vcx, "confirm-item-499".into()).is_none());
}

#[gpui::test]
fn the_context_names_the_cluster_the_initiator_and_a_missing_preview(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let prompt = ConfirmPrompt::new("Delete the namespace?", "Deletes demo.")
        .with_cluster(
            "prod-eu",
            Some(oxikube_domain::ClusterColour::rgb(200, 30, 30)),
        )
        .with_initiator(Initiator::Agent)
        .with_risk(Some(Risk::Irreversible))
        .with_preview(Some(PreviewNote::Unavailable(
            "dry run not supported".into(),
        )));
    let (_dialog, _answers) = open_dialog(&ws, &mut vcx, prompt);
    for selector in [
        "confirm-cluster",
        "confirm-cluster-dot",
        "confirm-initiator",
        "confirm-risk",
        "confirm-preview",
    ] {
        assert!(bounds(&mut vcx, selector).is_some(), "{selector}");
    }
}

#[gpui::test]
fn the_older_dialog_modal_compares_exactly_too(cx: &mut TestAppContext) {
    use crate::modal::DialogModal;

    let (ws, mut vcx) = workspace(cx);
    let dialog = vcx.update(|window, cx| {
        let dialog =
            cx.new(|cx| DialogModal::new("Delete?", cx).require_typed("web-0", window, cx));
        ws.update(cx, |ws, cx| ws.show_modal(dialog.clone(), window, cx));
        dialog
    });
    for (typed, ok) in [
        ("web-0", true),
        (" web-0", false),
        ("web-0 ", false),
        ("WEB-0", false),
    ] {
        vcx.update(|window, cx| dialog.update(cx, |d, cx| d.type_text(typed, window, cx)));
        vcx.run_until_parked();
        assert_eq!(
            vcx.update(|_, cx| dialog.read(cx).can_confirm(cx)),
            ok,
            "{typed:?}"
        );
    }
}
