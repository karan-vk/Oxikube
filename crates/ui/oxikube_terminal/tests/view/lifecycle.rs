//! The terminal's lifecycle in the tab (E09-S12): a dropped connection and an exited shell show a
//! banner above a screen that stays visible, input is dimmed and stops, and Reconnect / Restart
//! start a new session from the same descriptor. Over `FakeTerminalBackend`.

use gpui::{Entity, Modifiers};
use oxikube_domain::OxiError;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_ports::ExitStatus;
use oxikube_terminal::view::{
    BackendDescriptor, BannerAction, Failure, FailureKind, Lifecycle, TerminalView,
};

use super::*;

pub(super) fn pod_shell() -> BackendDescriptor {
    BackendDescriptor::Exec {
        pod: ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "shop", "web-0"),
        container: Some("app".into()),
        command: vec!["/bin/sh".into()],
    }
}

/// The command `pod_shell()` is opened with.
pub(super) fn pod_shell_command() -> Command {
    Command::PodExec {
        target: ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "shop", "web-0"),
        container: Some("app".into()),
        command: vec!["/bin/sh".into()],
    }
}

fn local_shell() -> BackendDescriptor {
    BackendDescriptor::local(None).with_shell("/bin/zsh", vec!["-l".into()])
}

impl Harness {
    /// Clicks the element drawn under `selector`.
    fn click(&mut self, selector: &'static str) {
        self.vcx.update(|window, cx| window.draw(cx).clear(cx));
        let bounds = self
            .vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not drawn"));
        self.vcx
            .simulate_click(bounds.center(), Modifiers::default());
        self.vcx.run_until_parked();
    }

    fn lifecycle(&mut self, view: &Entity<TerminalView>) -> Lifecycle {
        self.vcx.update(|_, cx| view.read(cx).lifecycle().clone())
    }

    fn commands(&self) -> Vec<Command> {
        self.recorder.0.borrow().clone()
    }

    fn reconnect(&mut self, view: &Entity<TerminalView>) -> bool {
        let started = self
            .vcx
            .update(|_, cx| view.update(cx, |v, cx| v.reconnect(cx)));
        self.vcx.run_until_parked();
        started
    }

    fn restart(&mut self, view: &Entity<TerminalView>) -> bool {
        let started = self
            .vcx
            .update(|_, cx| view.update(cx, |v, cx| v.restart(cx)));
        self.vcx.run_until_parked();
        started
    }
}

#[gpui::test]
fn a_dropped_connection_shows_a_banner_over_the_kept_screen_and_reconnect_starts_over(
    cx: &mut TestAppContext,
) {
    let mut h = harness(cx);
    let view = h.open(pod_shell());
    h.frame();
    h.backend(0).output("hello from the pod\r\n");
    h.frame();
    assert_eq!(h.row(&view, 0), "hello from the pod");
    assert!(!h.drawn("terminal-banner"), "nothing to say while it runs");
    assert!(!h.drawn("terminal-dimmed"));

    // The websocket drops: the transport error, then the stream ends.
    h.backend(0).error(OxiError::network("connection reset"));
    h.backend(0).exit(ExitStatus::default());
    h.frame();

    assert_eq!(
        h.lifecycle(&view),
        Lifecycle::Disconnected(Failure::from_error(&OxiError::network("connection reset")))
    );
    let banner = h
        .vcx
        .update(|_, cx| view.read(cx).banner())
        .expect("a banner");
    assert_eq!(banner.headline, "Connection lost");
    assert_eq!(banner.actions, [BannerAction::Reconnect]);
    assert!(h.drawn("terminal-banner"), "the banner is above the grid");
    assert!(
        h.drawn("terminal-banner-Reconnect"),
        "with a Reconnect button"
    );
    assert!(h.drawn("terminal-dimmed"), "input is dimmed");
    assert_eq!(
        h.row(&view, 0),
        "hello from the pod",
        "the grid stays visible"
    );
    let dirty = h.vcx.update(|_, cx| {
        use oxikube_workspace::Item as _;
        view.read(cx).tab_content(cx).dirty
    });
    assert!(!dirty, "nothing runs any more");

    // Typing goes nowhere.
    h.vcx.simulate_keystrokes("l s enter");
    h.vcx.run_until_parked();
    assert!(
        h.backend(0).written().is_empty(),
        "no input reaches a dead session"
    );

    // The button sends the command; the window's views apply it to this terminal.
    h.click("terminal-banner-Reconnect");
    assert_eq!(h.commands(), [Command::TerminalReconnect]);
    assert_eq!(h.launches().len(), 1, "the command alone started nothing");
    assert!(h.reconnect(&view), "a dropped pod session reconnects");

    // A pod session is only started by its command: the guard checks read-only mode and audits
    // it, so the view sends `pod::Exec` again and starts nothing around the bus.
    assert_eq!(
        h.commands(),
        [Command::TerminalReconnect, pod_shell_command()],
        "the same target again, as its command"
    );
    assert_eq!(
        h.launches().len(),
        1,
        "no launch around the guard (the command's terminal opens the new session)"
    );
    assert!(
        h.backend(0).kill_count() >= 1,
        "the old session was torn down"
    );
    h.frame();
    assert!(
        h.drawn("terminal-banner") && h.drawn("terminal-dimmed"),
        "the old tab keeps its screen, in case the guard refuses"
    );
    assert_eq!(h.row(&view, 0), "hello from the pod");
}

#[gpui::test]
fn a_local_shell_exit_shows_its_code_and_restart_over_the_kept_screen(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let view = h.open(local_shell());
    h.frame();
    h.backend(0).output("bye\r\n");
    h.backend(0).exit(ExitStatus::with_code(2));
    h.frame();

    let banner = h
        .vcx
        .update(|_, cx| view.read(cx).banner())
        .expect("a banner");
    assert_eq!(banner.headline, "Shell exited with code 2");
    assert_eq!(
        banner.actions,
        [BannerAction::Restart, BannerAction::CloseTab]
    );
    assert!(h.drawn("terminal-banner-Restart"));
    assert!(h.drawn("terminal-banner-CloseTab"));
    assert!(
        !h.drawn("terminal-banner-Reconnect"),
        "a shell restarts, it does not reconnect"
    );
    assert_eq!(h.row(&view, 0), "bye", "the screen stays visible");

    h.click("terminal-banner-Restart");
    assert_eq!(h.commands(), [Command::TerminalRestart]);
    assert!(
        !h.reconnect(&view),
        "a local shell has nothing to reconnect to"
    );
    assert!(h.restart(&view));
    assert_eq!(
        h.launches(),
        [local_shell(), local_shell()],
        "same descriptor"
    );
    assert_eq!(h.lifecycle(&view), Lifecycle::Running);
    h.frame();
    assert!(!h.drawn("terminal-banner"));
}

#[gpui::test]
fn a_clean_exit_offers_close_tab_first(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let view = h.open(local_shell());
    h.backend(0).exit(ExitStatus::success());
    h.frame();
    let banner = h
        .vcx
        .update(|_, cx| view.read(cx).banner())
        .expect("a banner");
    assert_eq!(banner.headline, "Shell exited with code 0");
    assert_eq!(
        banner.actions,
        [BannerAction::CloseTab, BannerAction::Restart]
    );
    h.click("terminal-banner-CloseTab");
    assert_eq!(h.commands(), [Command::TerminalClose]);
}

#[gpui::test]
fn a_running_terminal_does_not_restart_or_reconnect(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let local = h.open(local_shell());
    let pod = h.open(pod_shell());
    assert!(!h.restart(&local));
    assert!(!h.reconnect(&pod));
    assert_eq!(h.launches().len(), 2, "nothing was started again");
    assert_eq!(h.backend(0).kill_count(), 0);
    assert_eq!(h.backend(1).kill_count(), 0);
}

#[gpui::test]
fn a_pod_that_cannot_be_opened_says_why_and_reconnect_retries(cx: &mut TestAppContext) {
    let launcher = FakeLauncher::default();
    *launcher.fail_next.borrow_mut() =
        Some(OxiError::forbidden("not allowed to exec in pod shop/web-0"));
    let mut h = harness_with(cx, launcher);
    let view = h.open(pod_shell());
    h.frame();

    let lifecycle = h.lifecycle(&view);
    assert!(
        matches!(&lifecycle, Lifecycle::Failed(failure) if failure.kind() == FailureKind::Forbidden),
        "{lifecycle:?}"
    );
    let banner = h
        .vcx
        .update(|_, cx| view.read(cx).banner())
        .expect("a banner");
    assert_eq!(banner.headline, "Not allowed to open a terminal here");
    assert!(
        !banner.detail.contains("shop/web-0"),
        "the sentence is plain: {}",
        banner.detail
    );
    assert_eq!(
        banner.details.as_deref(),
        Some("not allowed to exec in pod shop/web-0"),
        "the server's message is behind Details"
    );
    // Details starts collapsed, opens the raw text and closes it again.
    assert!(h.drawn("terminal-banner-details-toggle"));
    assert!(!h.drawn("terminal-banner-details"));
    h.click("terminal-banner-details-toggle");
    assert!(h.drawn("terminal-banner-details"));
    h.click("terminal-banner-details-toggle");
    assert!(!h.drawn("terminal-banner-details"));
    assert!(h.drawn("terminal-banner-Reconnect"));
    assert!(h.drawn("terminal-failed"), "the backdrop stays");

    // The user's role was fixed: Reconnect asks for the session through its command again (the
    // guard applies its policy and audits it); the failed tab, with no screen to keep, closes.
    assert!(h.reconnect(&view));
    assert_eq!(h.commands(), [pod_shell_command()]);
    assert_eq!(h.launches().len(), 1, "no launch around the guard");
    assert!(
        super::commands::terminals(&mut h).is_empty(),
        "the failed tab closed"
    );
}

#[gpui::test]
fn a_pod_terminal_without_a_command_dispatcher_does_not_reconnect(cx: &mut TestAppContext) {
    let launcher = FakeLauncher::default();
    *launcher.fail_next.borrow_mut() = Some(OxiError::network("connection reset"));
    let mut h = harness_with(cx, launcher);
    // A bare view: no bus behind it, so nothing could check the policy or audit an open.
    let services = TerminalServices::new(h.launcher.clone());
    let view = h
        .vcx
        .update(|_, cx| cx.new(|cx| TerminalView::new(pod_shell(), services, cx)));
    h.vcx.run_until_parked();
    assert!(
        !h.reconnect(&view),
        "a session is never opened around the bus"
    );
    assert_eq!(h.launches().len(), 1);
}

/// The kinds the banner distinguishes, from a failed pod start.
#[gpui::test]
fn every_failed_start_gets_its_own_banner(cx: &mut TestAppContext) {
    let cases = [
        (
            OxiError::auth("expired", true),
            FailureKind::AuthExpired,
            "Authentication expired",
        ),
        (
            OxiError::not_found("pod shop/web-0 not found"),
            FailureKind::PodGone,
            "The pod or container is gone",
        ),
        (
            OxiError::conflict("pod is Succeeded"),
            FailureKind::ContainerStopped,
            "The container is not running",
        ),
        (
            OxiError::timeout("timed out"),
            FailureKind::ConnectionLost,
            "Connection lost",
        ),
    ];
    for (error, kind, headline) in cases {
        let launcher = FakeLauncher::default();
        *launcher.fail_next.borrow_mut() = Some(error);
        let mut h = harness_with(cx, launcher);
        let view = h.open(pod_shell());
        let banner = h
            .vcx
            .update(|_, cx| view.read(cx).banner())
            .expect("a banner");
        assert_eq!(banner.headline, headline);
        let lifecycle = h.lifecycle(&view);
        assert!(
            matches!(&lifecycle, Lifecycle::Failed(failure) if failure.kind() == kind),
            "{lifecycle:?}"
        );
    }
}

#[gpui::test]
fn a_pod_that_is_gone_offers_close_instead_of_a_futile_reconnect(cx: &mut TestAppContext) {
    let launcher = FakeLauncher::default();
    *launcher.fail_next.borrow_mut() = Some(OxiError::not_found("pods \"web-0\" not found"));
    let mut h = harness_with(cx, launcher);
    let view = h.open(pod_shell());
    h.frame();

    let banner = h
        .vcx
        .update(|_, cx| view.read(cx).banner())
        .expect("a banner");
    assert_eq!(banner.headline, "The pod or container is gone");
    assert_eq!(banner.actions, [BannerAction::CloseTab]);
    assert!(h.drawn("terminal-banner-CloseTab"));
    assert!(
        !h.drawn("terminal-banner-Reconnect"),
        "reconnecting to something that no longer exists cannot work"
    );
    // The raw server text is behind Details, the sentence is plain.
    assert_eq!(banner.details.as_deref(), Some("pods \"web-0\" not found"));
    assert!(h.drawn("terminal-banner-details-toggle"));

    h.click("terminal-banner-CloseTab");
    assert_eq!(h.commands(), [Command::TerminalClose]);
}
