//! Closing a terminal releases everything it started (E09-S12): the backend task, the reader and
//! writer, the session and the view. Counted three ways: `oxikube_runtime::live_tasks` (the
//! bridge's futures), a backend that counts itself while alive, and weak handles.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpui::{EntityId, WeakEntity};
use oxikube_ports::ExitStatus;
use oxikube_terminal::TerminalState;
use oxikube_terminal::view::{BackendDescriptor, TerminalView};

use super::lifecycle::pod_shell;
use super::*;

fn launcher_with_probe() -> (FakeLauncher, Arc<AtomicUsize>) {
    let alive = Arc::new(AtomicUsize::new(0));
    let launcher = FakeLauncher {
        alive: Some(alive.clone()),
        ..FakeLauncher::default()
    };
    (launcher, alive)
}

impl Harness {
    fn live_tasks(&mut self) -> usize {
        self.vcx.update(|_, cx| oxikube_runtime::live_tasks(cx))
    }

    fn close(&mut self, id: EntityId) {
        let ws = self.ws.clone();
        self.vcx
            .update(|window, cx| ws.update(cx, |ws, cx| ws.close_item(id, window, cx)));
        self.frame();
    }
}

/// The resident set of this process in KiB (`ps`), for the numbers the test prints.
fn rss_kib() -> u64 {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

#[gpui::test]
fn closing_a_tab_during_output_aborts_the_stream_and_releases_the_backend(cx: &mut TestAppContext) {
    let (launcher, alive) = launcher_with_probe();
    let mut h = harness_with(cx, launcher);
    let baseline = h.live_tasks();
    let view = h.open(pod_shell());
    h.frame();
    assert!(h.live_tasks() > baseline, "the pump and the writer run");
    assert_eq!(alive.load(Ordering::SeqCst), 1);

    // The process is busy: output keeps coming while the tab closes.
    for _ in 0..20 {
        h.backend(0).output("x".repeat(2048));
    }
    let state: WeakEntity<TerminalState> = h
        .vcx
        .update(|_, cx| view.read(cx).terminal().expect("running").downgrade());
    h.close(view.entity_id());

    let backend = h.backend(0);
    assert_eq!(backend.kill_count(), 1, "the session was ended");
    assert!(backend.is_closed());
    assert!(
        !backend.output("more output after the close"),
        "nothing is left to take it"
    );
    assert!(state.upgrade().is_none(), "the session is released");
    assert_eq!(
        h.live_tasks(),
        baseline,
        "the pump, writer and kill are gone"
    );
    drop(view);
    h.vcx.run_until_parked();
    assert_eq!(
        alive.load(Ordering::SeqCst),
        0,
        "no backend remains referenced"
    );
}

#[gpui::test]
fn dropping_the_view_without_closing_the_tab_releases_it_too(cx: &mut TestAppContext) {
    let (launcher, alive) = launcher_with_probe();
    let mut h = harness_with(cx, launcher);
    let baseline = h.live_tasks();
    let view = h.terminal(pod_shell());
    assert_eq!(alive.load(Ordering::SeqCst), 1);
    assert!(h.live_tasks() > baseline);
    let weak = view.downgrade();

    drop(view);
    // Released entities are collected at the next update: the view first, then its session.
    for _ in 0..3 {
        h.vcx.update(|_, _| {});
        h.vcx.run_until_parked();
    }

    assert!(weak.upgrade().is_none());
    assert_eq!(h.live_tasks(), baseline, "tasks die with the view");
    assert_eq!(alive.load(Ordering::SeqCst), 0, "and so does the backend");
}

/// 50 open/close cycles, some through a dropped connection, a reconnect, an exit and a restart:
/// the task count, the live backends and the views all return to where they started.
#[gpui::test]
fn fifty_open_close_cycles_leave_nothing_behind(cx: &mut TestAppContext) {
    let (launcher, alive) = launcher_with_probe();
    let mut h = harness_with(cx, launcher);
    let baseline = h.live_tasks();
    let rss_before = rss_kib();
    let mut views: Vec<WeakEntity<TerminalView>> = Vec::new();
    let mut peak = baseline;

    for cycle in 0..50 {
        let pod = cycle % 2 == 0;
        let descriptor = if pod {
            pod_shell()
        } else {
            BackendDescriptor::local(None).with_shell("/bin/zsh", vec![])
        };
        let view = h.open(descriptor);
        h.frame();
        let first = h.launches().len() - 1;
        h.backend(first).output("some output\r\n".repeat(50));
        h.frame();
        if cycle % 5 == 0 {
            // Lose the session, bring it back, then close.
            if pod {
                h.backend(first)
                    .error(oxikube_domain::OxiError::network("reset"));
                h.backend(first).exit(ExitStatus::default());
            } else {
                h.backend(first).exit(ExitStatus::with_code(1));
            }
            h.frame();
            let again = h
                .vcx
                .update(|_, cx| view.update(cx, |view, cx| view.reconnect(cx) || view.restart(cx)));
            assert!(again, "cycle {cycle} starts a new session");
            h.frame();
        }
        peak = peak.max(h.live_tasks());
        views.push(view.downgrade());
        h.close(view.entity_id());
        drop(view);
        h.vcx.run_until_parked();
        assert_eq!(
            h.live_tasks(),
            baseline,
            "cycle {cycle}: tasks back to baseline"
        );
    }

    assert_eq!(h.live_tasks(), baseline, "no task growth after 50 cycles");
    assert_eq!(alive.load(Ordering::SeqCst), 0, "no backend remains alive");
    assert!(
        views.iter().all(|view| view.upgrade().is_none()),
        "every view was released"
    );
    assert!(peak > baseline, "the cycles did start tasks (peak {peak})");
    let rss_after = rss_kib();
    eprintln!(
        "terminal leak test: live tasks baseline {baseline}, peak in a cycle {peak}, after 50 cycles {}; RSS {rss_before} KiB -> {rss_after} KiB",
        h.live_tasks()
    );
}
