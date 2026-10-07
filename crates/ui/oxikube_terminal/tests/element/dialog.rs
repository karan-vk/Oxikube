//! The multi-line paste confirmation as the workspace's modal dialog.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{AppContext as _, TestAppContext, VisualTestContext};
use oxikube_terminal::input::{PasteConfirm, WorkspacePasteConfirm};
use oxikube_ui::root::Root;
use oxikube_workspace::{DialogModal, Workspace};

fn workspace_window(cx: &mut TestAppContext) -> (gpui::Entity<Workspace>, VisualTestContext) {
    cx.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_workspace::init(cx);
    });
    let mut workspace = None;
    let window = cx.add_window(|window, cx| {
        let entity = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.update(|window, _| window.activate_window());
    vcx.run_until_parked();
    (workspace.expect("the window was built"), vcx)
}

fn ask(
    workspace: &gpui::Entity<Workspace>,
    vcx: &mut VisualTestContext,
    text: &'static str,
) -> Rc<Cell<u32>> {
    let accepted = Rc::new(Cell::new(0));
    let confirm = WorkspacePasteConfirm::new(workspace.downgrade());
    let counter = accepted.clone();
    vcx.update(|window, cx| {
        confirm.confirm(
            text,
            Rc::new(move |_, _| counter.set(counter.get() + 1)),
            window,
            cx,
        )
    });
    vcx.run_until_parked();
    accepted
}

fn dialog(
    workspace: &gpui::Entity<Workspace>,
    vcx: &mut VisualTestContext,
) -> Option<(String, Option<String>)> {
    vcx.read(|cx| {
        let layer = workspace.read(cx).modal_layer().read(cx);
        layer.active_modal::<DialogModal>().map(|dialog| {
            let dialog = dialog.read(cx);
            (
                dialog.title().to_string(),
                dialog.message_text().map(ToString::to_string),
            )
        })
    })
}

#[gpui::test]
fn the_dialog_previews_the_first_lines_and_waits(cx: &mut TestAppContext) {
    let (workspace, mut vcx) = workspace_window(cx);
    let accepted = ask(&workspace, &mut vcx, "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\n");
    let (title, message) = dialog(&workspace, &mut vcx).expect("the dialog is open");
    assert_eq!(title, "Paste 9 lines into the terminal?");
    assert_eq!(
        message.as_deref(),
        Some("l1\nl2\nl3\nl4\nl5\nl6\n… and 3 more lines")
    );
    assert_eq!(
        accepted.get(),
        0,
        "nothing is pasted before the user confirms"
    );
}

#[gpui::test]
fn confirming_pastes_and_cancelling_does_not(cx: &mut TestAppContext) {
    let (workspace, mut vcx) = workspace_window(cx);
    let accepted = ask(&workspace, &mut vcx, "a\nb");
    vcx.simulate_keystrokes("escape");
    assert_eq!(
        dialog(&workspace, &mut vcx),
        None,
        "escape closes the dialog"
    );
    assert_eq!(accepted.get(), 0);

    let accepted = ask(&workspace, &mut vcx, "a\nb");
    vcx.simulate_keystrokes("enter");
    assert_eq!(dialog(&workspace, &mut vcx), None);
    assert_eq!(accepted.get(), 1, "enter confirms");
}
