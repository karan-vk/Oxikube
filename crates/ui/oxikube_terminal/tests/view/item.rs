//! The terminal tab: title from the program, then from the process; dirty while it runs; the
//! cluster's mark; the exit line; a failed start; closing ends the process and releases it.

use std::sync::Arc;

use gpui::{Entity, EntityId};
use oxikube_app::ClusterSessionManager;
use oxikube_app::session::SessionOptions;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::{ClusterPreset, OxiError};
use oxikube_ports::{ClusterContext, ExitStatus, SourceId};
use oxikube_terminal::TerminalState;
use oxikube_terminal::view::{BackendDescriptor, LocalLauncher, TerminalView};
use oxikube_testkit::{FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort};
use oxikube_ui::IconName;
use oxikube_workspace::{ClusterMark, TabContent};

use super::*;

fn tab(h: &mut Harness, view: &Entity<TerminalView>) -> TabContent {
    h.vcx.update(|_, cx| {
        use oxikube_workspace::Item as _;
        view.read(cx).tab_content(cx)
    })
}

fn zsh() -> BackendDescriptor {
    BackendDescriptor::local(None).with_shell("/bin/zsh", vec!["-l".into()])
}

#[gpui::test]
fn the_tab_shows_the_program_then_the_process_title_and_is_dirty_while_running(
    cx: &mut TestAppContext,
) {
    let mut h = harness(cx);
    let view = h.open(zsh());

    assert_eq!(h.launches(), [zsh()], "started once, from the descriptor");
    let content = tab(&mut h, &view);
    assert_eq!(content.title.as_ref(), "zsh");
    assert_eq!(content.icon, Some(IconName::Terminal));
    assert!(content.dirty, "a process runs");
    assert_eq!(content.cluster, None);

    // The process sets its title (OSC 0); the tab follows.
    h.backend(0).output("\x1b]0;vim main.rs\x07");
    h.frame();
    assert_eq!(tab(&mut h, &view).title.as_ref(), "vim main.rs");
    h.backend(0).output("\x1b]0;\x07");
    h.frame();
    assert_eq!(tab(&mut h, &view).title.as_ref(), "zsh", "a reset title");

    h.backend(0).exit(ExitStatus::with_code(3));
    h.frame();
    assert!(!tab(&mut h, &view).dirty, "nothing runs any more");
    assert!(h.drawn("terminal-banner"), "the exit banner shows");
    let status = h
        .vcx
        .update(|_, cx| view.read(cx).exit_status(cx).cloned())
        .expect("exited");
    assert_eq!(status, ExitStatus::with_code(3));
}

#[gpui::test]
fn a_cluster_or_pod_terminal_carries_the_cluster_mark_and_its_name(cx: &mut TestAppContext) {
    let mark = ClusterMark {
        colour: None,
        read_only: true,
    };
    let mut h = harness_with(
        cx,
        FakeLauncher {
            mark: Some(mark),
            ..FakeLauncher::default()
        },
    );
    let local = h.open(BackendDescriptor::local(Some(cluster())).with_shell("bash", vec![]));
    assert_eq!(tab(&mut h, &local).cluster, Some(mark));

    let pod = ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "shop", "web-0");
    let exec = h.open(BackendDescriptor::Exec {
        pod,
        container: Some("app".into()),
        command: vec!["/bin/sh".into()],
    });
    let content = tab(&mut h, &exec);
    assert_eq!(content.title.as_ref(), "web-0/app", "the pod's name");
    assert_eq!(content.icon, Some(IconName::Container));
    assert_eq!(content.cluster, Some(mark));
}

/// The app's launcher over a session manager with `cluster()` open: the tab's mark comes from
/// the session and follows its read-only flag and colour after the terminal opened.
#[gpui::test]
fn the_cluster_mark_follows_the_session_after_the_terminal_opened(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let clock = Arc::new(FakeClockPort::default());
    let source = Arc::new(FakeClusterSourcePort::new());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        source.clone(),
        clock,
    );
    let context = ClusterContext::new(
        cluster(),
        ContextName::new("kind-dev"),
        SourceId("kubeconfig".into()),
    );
    sessions.open(&context, SessionOptions::default());
    let services = TerminalServices::new(Rc::new(LocalLauncher::new(sessions.clone(), source)));
    // A pod terminal: this launcher does not start those (no process in this test), but its tab
    // still carries the pod's cluster mark.
    let pod = ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "shop", "web-0");
    let descriptor = BackendDescriptor::Attach {
        pod,
        container: None,
    };
    let view = h
        .vcx
        .update(|_, cx| cx.new(|cx| TerminalView::new(descriptor, services, cx)));
    h.vcx.run_until_parked();
    assert_eq!(
        tab(&mut h, &view).cluster,
        Some(ClusterMark::default()),
        "writable, no colour"
    );

    sessions.set_read_only(&cluster(), true).expect("open");
    h.vcx.run_until_parked();
    assert_eq!(
        tab(&mut h, &view).cluster,
        Some(ClusterMark {
            colour: None,
            read_only: true
        }),
        "the lock shows once the cluster turned read-only"
    );

    sessions
        .set_colour(&cluster(), Some(ClusterPreset::PROD_COLOUR))
        .expect("open");
    sessions.set_read_only(&cluster(), false).expect("open");
    h.vcx.run_until_parked();
    assert_eq!(
        tab(&mut h, &view).cluster,
        Some(ClusterMark {
            colour: Some(ClusterPreset::PROD_COLOUR),
            read_only: false
        }),
        "the new colour, and the lock gone"
    );

    sessions.close(&cluster());
    h.vcx.run_until_parked();
    assert_eq!(
        tab(&mut h, &view).cluster,
        None,
        "a closed session, no mark"
    );
}

#[gpui::test]
fn a_terminal_that_cannot_start_says_why_and_is_not_dirty(cx: &mut TestAppContext) {
    let launcher = FakeLauncher::default();
    *launcher.fail_next.borrow_mut() = Some(OxiError::validation("no such shell: /bin/nope"));
    let mut h = harness_with(cx, launcher);
    let view = h.open(zsh());
    let failure = h.vcx.update(|_, cx| view.read(cx).failure().cloned());
    assert!(
        failure.is_some_and(|message| message.contains("no such shell")),
        "the reason is kept"
    );
    assert!(!tab(&mut h, &view).dirty);
    assert!(h.drawn("terminal-failed"));
}

#[gpui::test]
fn closing_the_tab_kills_the_process_and_releases_the_session(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let view = h.open(zsh());
    let state: gpui::WeakEntity<TerminalState> = h
        .vcx
        .update(|_, cx| view.read(cx).terminal().expect("running").downgrade());
    let id: EntityId = view.entity_id();
    let ws = h.ws.clone();

    let closed = h
        .vcx
        .update(|window, cx| ws.update(cx, |ws, cx| ws.close_item(id, window, cx)));
    h.frame();

    assert!(closed);
    let backend = h.backend(0);
    assert_eq!(backend.kill_count(), 1, "the process was ended");
    assert!(backend.is_closed());
    assert!(
        state.upgrade().is_none(),
        "the session (grid, pump and writer tasks) is gone with the tab"
    );
    assert!(h.vcx.update(|_, cx| view.read(cx).terminal().is_none()));
}

#[gpui::test]
fn the_terminal_takes_the_focus_when_its_tab_is_shown(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let view = h.open(zsh());
    h.frame();
    let focused = h.vcx.update(|window, cx| {
        use gpui::Focusable as _;
        view.read(cx).focus_handle(cx).is_focused(window)
    });
    assert!(focused, "keystrokes go to the terminal");
    // And they reach the process.
    h.vcx.simulate_keystrokes("l s enter");
    h.vcx.run_until_parked();
    assert_eq!(h.backend(0).written(), b"ls\r");
}
