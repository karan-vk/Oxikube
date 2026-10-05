//! Quit confirmation: a running operation shows the modal, cancel keeps the app, nothing running
//! quits at once.

use gpui::{TestAppContext, VisualTestContext};
use oxikube_settings::update_user_settings;
use oxikube_ui::dialog::OverlayExt as _;

use super::{count_quits, open_window, setup};
use crate::session::{
    Quit, RunningOperation, SessionSettings,
    quit::{needs_confirmation, operation_lines},
    register_operation_provider, request_quit, running_operations, unregister_operation_provider,
    windows::{allow_close, should_close},
};
use std::{cell::RefCell, rc::Rc};

type Running = Rc<RefCell<Vec<RunningOperation>>>;

/// Registers a provider whose list the test edits through the returned handle.
fn provider(cx: &mut TestAppContext) -> Running {
    let running: Running = Rc::default();
    let shared = running.clone();
    cx.update(|cx| register_operation_provider(cx, move |_| shared.borrow().clone()));
    running
}

fn exec_session() -> RunningOperation {
    RunningOperation::new("Exec session", "pod/web-0 in prod")
}

fn dialog_open(vcx: &mut VisualTestContext) -> bool {
    vcx.update(|window, cx| window.has_active_dialog(cx))
}

#[gpui::test]
fn quit_with_nothing_running_quits_at_once(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let quits = count_quits(cx);
    let (_handle, mut vcx) = open_window(cx);
    vcx.dispatch_action(Quit);
    vcx.run_until_parked();
    assert_eq!(quits.get(), 1);
    assert!(!dialog_open(&mut vcx), "no dialog when nothing is running");
}

#[gpui::test]
fn quit_with_a_running_operation_shows_the_modal_and_does_not_quit(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let quits = count_quits(cx);
    let running = provider(cx);
    running.borrow_mut().push(exec_session());
    let (_handle, mut vcx) = open_window(cx);

    vcx.dispatch_action(Quit);
    vcx.run_until_parked();
    assert!(dialog_open(&mut vcx), "the confirm modal is up");
    assert_eq!(quits.get(), 0, "and the app is still running");
    assert!(
        vcx.debug_bounds("dialog-layer").is_some(),
        "rendered by the Root's dialog layer"
    );
}

#[gpui::test]
fn cancel_keeps_the_app_and_confirm_quits(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let quits = count_quits(cx);
    let running = provider(cx);
    running.borrow_mut().push(exec_session());
    let (_handle, mut vcx) = open_window(cx);

    vcx.dispatch_action(Quit);
    vcx.run_until_parked();
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(!dialog_open(&mut vcx), "cancel closes the modal");
    assert_eq!(quits.get(), 0, "and keeps the app");
    assert_eq!(cx.update(|cx| cx.windows().len()), 1);

    // Asking again works, and confirming quits.
    vcx.dispatch_action(Quit);
    vcx.run_until_parked();
    assert!(dialog_open(&mut vcx));
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(quits.get(), 1, "confirm quits once");
}

#[gpui::test]
fn repeated_quit_requests_do_not_stack_dialogs(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let _quits = count_quits(cx);
    let running = provider(cx);
    running.borrow_mut().push(exec_session());
    let (_handle, mut vcx) = open_window(cx);
    for _ in 0..3 {
        vcx.dispatch_action(Quit);
        vcx.run_until_parked();
    }
    vcx.update(|window, cx| window.close_dialog(cx));
    assert!(!dialog_open(&mut vcx), "one dialog was open, not three");
}

#[gpui::test]
fn the_providers_decide_when_to_ask(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let first = provider(cx);
    let second = provider(cx);
    assert!(!cx.update(|cx| needs_confirmation(cx)));
    second
        .borrow_mut()
        .push(RunningOperation::new("Port-forward", "svc/db :5432"));
    first.borrow_mut().push(exec_session());
    let all = cx.update(|cx| running_operations(cx));
    assert_eq!(all.len(), 2, "every provider is asked");
    assert!(cx.update(|cx| needs_confirmation(cx)));

    // An unregistered provider is no longer asked.
    let id = cx.update(|cx| register_operation_provider(cx, |_| vec![exec_session()]));
    assert_eq!(cx.update(|cx| running_operations(cx)).len(), 3);
    cx.update(|cx| unregister_operation_provider(cx, id));
    assert_eq!(cx.update(|cx| running_operations(cx)).len(), 2);
}

#[gpui::test]
fn quit_confirmation_is_configurable(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let quits = count_quits(cx);
    let running = provider(cx);
    running.borrow_mut().push(exec_session());
    let (_handle, mut vcx) = open_window(cx);
    cx.update(|cx| {
        update_user_settings::<SessionSettings>(cx, None, |c| c.confirm_quit = Some(false))
            .detach();
    });
    vcx.run_until_parked();
    assert!(!cx.update(|cx| needs_confirmation(cx)));
    cx.update(request_quit);
    vcx.run_until_parked();
    assert_eq!(quits.get(), 1, "no modal with confirm_quit off");
    assert!(!dialog_open(&mut vcx));
}

#[gpui::test]
fn quitting_with_no_window_reopens_one_to_ask(cx: &mut TestAppContext) {
    // macOS keeps the app alive after its last window closes; operations may still be running.
    let _dir = setup(cx);
    let quits = count_quits(cx);
    let running = provider(cx);
    running.borrow_mut().push(exec_session());
    assert_eq!(cx.update(|cx| cx.windows().len()), 0);

    cx.update(request_quit);
    cx.run_until_parked();
    assert_eq!(quits.get(), 0, "not quit without asking");
    let windows = cx.update(|cx| cx.windows());
    assert_eq!(windows.len(), 1, "a window was opened for the question");
    let mut vcx = VisualTestContext::from_window(windows[0], cx);
    assert!(dialog_open(&mut vcx));

    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(quits.get(), 1, "confirming quits");
}

#[test]
fn the_dialog_lists_operations_and_summarises_the_rest() {
    let op = |n: usize| RunningOperation::new("Port-forward", format!("pod/p{n}"));
    let few: Vec<_> = (0..2).map(op).collect();
    assert_eq!(
        operation_lines(&few),
        ["Port-forward: pod/p0", "Port-forward: pod/p1"]
    );
    let many: Vec<_> = (0..11).map(op).collect();
    let lines = operation_lines(&many);
    assert_eq!(lines.len(), 9);
    assert_eq!(lines.last().map(String::as_str), Some("and 3 more"));
}

#[test]
fn closing_a_window_asks_only_when_it_would_quit_with_operations_running() {
    // (other windows, last close quits, needs confirmation) -> may close
    for (others, quits, needs, allowed) in [
        (true, true, true, true),
        (true, false, true, true),
        (false, false, true, true),
        (false, true, false, true),
        (false, true, true, false),
    ] {
        assert_eq!(
            allow_close(others, quits, needs),
            allowed,
            "others {others}, quits {quits}, needs {needs}"
        );
    }
}

#[gpui::test]
fn closing_the_last_window_asks_where_that_quits(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let quits = count_quits(cx);
    let running = provider(cx);
    let (_handle, mut vcx) = open_window(cx);

    // Nothing running: it just closes.
    assert!(vcx.update(|window, cx| should_close(window, cx, true)));

    running.borrow_mut().push(exec_session());
    // macOS keeps the app alive with no window: closing is fine.
    assert!(vcx.update(|window, cx| should_close(window, cx, false)));
    assert!(!dialog_open(&mut vcx));
    // Linux and Windows: closing the last window quits, so it asks instead and stays open.
    assert!(!vcx.update(|window, cx| should_close(window, cx, true)));
    vcx.run_until_parked();
    assert!(dialog_open(&mut vcx));
    assert_eq!(quits.get(), 0);
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(quits.get(), 1, "confirming quits");
}

#[gpui::test]
fn closing_one_of_several_windows_never_asks(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let running = provider(cx);
    running.borrow_mut().push(exec_session());
    let (_first, mut first_cx) = open_window(cx);
    let (_second, _second_cx) = open_window(cx);
    assert!(first_cx.update(|window, cx| should_close(window, cx, true)));
    assert!(!dialog_open(&mut first_cx));
}
