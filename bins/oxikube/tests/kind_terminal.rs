//! The terminal against a real cluster, through the app (E09-S13): a pod shell opened from the
//! pods table, a full-screen program running in it, the window resized, and what the app left on
//! disk afterwards.
//!
//! * a busybox `vi` draws in the grid (its `~` filler, its status line), the window is resized and
//!   the pod's tty follows (`stty size` in the shell after `vi` equals the grid), and `vi` repaints
//!   at the new size;
//! * `top` (the `htop` of busybox) repaints in place: the same rows are rewritten, nothing
//!   scrolls;
//! * a flood (`seq`) through the real websocket and the grid keeps the scrollback within its
//!   limit and reports its throughput;
//!
//! What the app leaves on disk is `kind_terminal_data`.
//!
//! `cargo test -p oxikube --features integration --test kind_terminal` with `OXIKUBE_TEST_CONTEXT`
//! set (`cargo xtask kind-up`); without it the test returns at once.
#![cfg(feature = "integration")]

mod kind_common;

use std::time::{Duration, Instant};

use gpui::{TestAppContext, VisualTestContext, px, size};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_terminal::view::TerminalView;
use oxikube_testkit::integration::{TestNamespace, ensure_kind_context, test_context};
use oxikube_ui::Unscaled;
use oxikube_workspace::{ClusterTab, DockPosition};

use kind_common::{
    Launched, create_pod, grid_size, launch, open_pod_shell, screen, type_into, wait,
};

/// The rows of the terminal's screen, trailing blanks trimmed.
fn rows(vcx: &mut VisualTestContext, terminal: &gpui::Entity<TerminalView>) -> Vec<String> {
    screen(vcx, terminal).lines().map(str::to_owned).collect()
}

/// How many rows of the screen are exactly `~`: vi's filler past the end of the file.
fn tildes(vcx: &mut VisualTestContext, terminal: &gpui::Entity<TerminalView>) -> usize {
    rows(vcx, terminal).iter().filter(|row| *row == "~").count()
}

/// Lines of history the terminal's grid holds above the screen.
fn history(vcx: &mut VisualTestContext, terminal: &gpui::Entity<TerminalView>) -> usize {
    vcx.update(|_, cx| {
        let state = terminal.read(cx).terminal().cloned().expect("running");
        state.read(cx).snapshot().history_size
    })
}

/// Widens the window by `by` pixels: the dock's terminal gets more columns.
fn widen_window(vcx: &mut VisualTestContext, by: f32) {
    let current = vcx.update(|window, _| window.viewport_size());
    vcx.simulate_resize(size(current.width + px(by), current.height));
}

/// Makes the cluster tab's bottom dock `height` (unscaled) pixels tall: the terminal gets more rows.
fn set_dock_height(vcx: &mut VisualTestContext, tab: &gpui::Entity<ClusterTab>, height: f32) {
    let inner = vcx.update(|_, cx| tab.read(cx).workspace().clone());
    vcx.update(|window, cx| {
        inner.update(cx, |ws, cx| {
            ws.resize_dock(DockPosition::Bottom, Unscaled(height), window, cx)
        })
    });
}

#[gpui::test]
fn a_full_screen_program_in_a_pod_renders_and_follows_a_resize(cx: &mut TestAppContext) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    cx.executor().allow_parking();
    let ns = TestNamespace::create(&context).expect("namespace");
    create_pod(&context, ns.name(), "screen");
    let Launched {
        mut vcx,
        tab,
        cluster,
        table,
        ..
    } = launch(cx, &context);
    let target = ResourceRef::namespaced(cluster, Gvk::new("", "v1", "Pod"), ns.name(), "screen");
    let terminal = open_pod_shell(&mut vcx, &tab, &table, &target);
    wait(&mut vcx, "the shell to open", |vcx| {
        screen(vcx, &terminal).contains("using sh")
    });

    // --- vi: the screen it draws, then the same screen at another size ---------------------------
    type_into(&mut vcx, &terminal, "vi /tmp/notes\n");
    wait(&mut vcx, "vi to draw", |vcx| {
        rows(vcx, &terminal)
            .last()
            .is_some_and(|row| row.starts_with("- /tmp/notes"))
    });
    let (columns, lines) = grid_size(&mut vcx, &terminal);
    assert_eq!(
        tildes(&mut vcx, &terminal),
        usize::from(lines) - 2,
        "a blank first row, `~` for the rest, the status line last"
    );
    type_into(&mut vcx, &terminal, "ihello from vi\x1b");
    wait(&mut vcx, "the typed text on the first row", |vcx| {
        rows(vcx, &terminal)[0] == "hello from vi"
    });

    widen_window(&mut vcx, 240.);
    set_dock_height(&mut vcx, &tab, 480.);
    wait(
        &mut vcx,
        "the grid to grow with the window and the dock",
        |vcx| {
            let (c, l) = grid_size(vcx, &terminal);
            c > columns && l > lines
        },
    );
    let (columns_after, lines_after) = grid_size(&mut vcx, &terminal);
    // vi gets SIGWINCH through the exec channel, asks the tty for its size and repaints.
    wait(&mut vcx, "vi to repaint at the new size", |vcx| {
        let screen = rows(vcx, &terminal);
        screen.len() == usize::from(lines_after)
            && tildes(vcx, &terminal) == usize::from(lines_after) - 2
            && screen[0] == "hello from vi"
            && screen
                .last()
                .is_some_and(|row| row.starts_with("- /tmp/notes"))
    });

    // Quit without saving; the shell's tty has the grid's size.
    type_into(&mut vcx, &terminal, ":q!\n");
    wait(&mut vcx, "vi to quit", |vcx| tildes(vcx, &terminal) == 0);
    type_into(&mut vcx, &terminal, "echo size=$(stty size)=end\n");
    let expected = format!("size={lines_after} {columns_after}=end");
    wait(&mut vcx, "the pod's tty to report the grid's size", |vcx| {
        screen(vcx, &terminal).contains(&expected)
    });

    // --- top: repaints in place, it never scrolls ------------------------------------------------
    type_into(&mut vcx, &terminal, "top -d 1\n");
    wait(&mut vcx, "top to draw its header", |vcx| {
        rows(vcx, &terminal)[0].starts_with("Mem:")
    });
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(3) {
        // Three refreshes at least: each rewrites the same rows from the top.
        vcx.run_until_parked();
        std::thread::sleep(Duration::from_millis(50));
    }
    let screen_now = rows(&mut vcx, &terminal);
    assert!(screen_now[0].starts_with("Mem:"), "{screen_now:?}");
    assert!(
        screen_now
            .iter()
            .any(|row| row.contains("PID") && row.contains("COMMAND")),
        "the process table header: {screen_now:?}"
    );
    assert_eq!(
        history(&mut vcx, &terminal),
        0,
        "top repaints, nothing scrolls into history"
    );
    // `q` quits; the shell's prompt is back, so a command's output shows below top's last frame.
    type_into(&mut vcx, &terminal, "q");
    type_into(&mut vcx, &terminal, "echo top-gone-$((1+1))\n");
    wait(&mut vcx, "the shell after top", |vcx| {
        screen(vcx, &terminal).contains("top-gone-2")
    });
}

#[gpui::test]
fn a_flood_of_output_through_the_websocket_stays_within_the_scrollback(cx: &mut TestAppContext) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    cx.executor().allow_parking();
    let ns = TestNamespace::create(&context).expect("namespace");
    create_pod(&context, ns.name(), "flood");
    let Launched {
        mut vcx,
        tab,
        cluster,
        table,
        ..
    } = launch(cx, &context);
    let target = ResourceRef::namespaced(cluster, Gvk::new("", "v1", "Pod"), ns.name(), "flood");
    let terminal = open_pod_shell(&mut vcx, &tab, &table, &target);
    wait(&mut vcx, "the shell to open", |vcx| {
        screen(vcx, &terminal).contains("using sh")
    });

    // 200 000 lines, 1.3 MB, as fast as the pod writes them; the marker is computed by the shell,
    // so only the real output satisfies the wait.
    const LINES: usize = 200_000;
    const BYTES: usize = 1_288_894;
    type_into(
        &mut vcx,
        &terminal,
        &format!("seq 1 {LINES}; echo flood-$((1+1))-done\n"),
    );
    let started = Instant::now();
    wait(&mut vcx, "the flood to end", |vcx| {
        screen(vcx, &terminal).contains("flood-2-done")
    });
    let elapsed = started.elapsed();
    let screen = rows(&mut vcx, &terminal);
    assert!(
        screen.iter().any(|row| row == &LINES.to_string()),
        "the last line of `seq` is on screen: {screen:?}"
    );
    let kept = history(&mut vcx, &terminal);
    assert!(
        (1..=oxikube_terminal::grid::DEFAULT_SCROLLBACK_LINES).contains(&kept),
        "scrollback is kept, and bounded by its limit: {kept}"
    );
    let megabytes = BYTES as f64 / 1_048_576.0;
    // The perf record the integration workflow keeps (`terminal-perf.jsonl`): numbers only.
    eprintln!(
        "perf-jsonl {}",
        serde_json::json!({
            "scenario": "terminal_flood_kind",
            "bytes": BYTES,
            "seconds": (elapsed.as_secs_f64() * 1000.0).round() / 1000.0,
            "mib_per_s": (megabytes / elapsed.as_secs_f64() * 10.0).round() / 10.0,
            "history_lines": kept,
        })
    );
    assert!(elapsed < Duration::from_secs(30), "{elapsed:?}");
}
