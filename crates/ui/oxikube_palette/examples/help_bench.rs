//! Micro benchmark of the help overlay (E11-S10): resolving the bindings in force for the focused
//! view (once per open), opening the overlay (the workspace's modal layer, the picker, the first
//! frame), and a keystroke in its search field, over the shipped keymap and over a keymap of 2 000
//! bindings.
//!
//! `cargo run -p oxikube_palette --features test-support --profile release-fast --example help_bench`
//!
//! Runs on GPUI's test platform (no GPU, test text system): a regression check against
//! docs/PERFORMANCE.md (palette: open <= 1 frame, filter 2 000 entries <= 5 ms), not the frame
//! budget itself.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::fmt::Write as _;
use std::rc::Rc;
use std::time::Instant;

use gpui::{Entity, FocusHandle, Focusable as _, TestAppContext, VisualTestContext};
use oxikube_keymap::KeymapOptions;
use oxikube_palette::help::{HelpHost, HelpModel, HelpOverlay};
use oxikube_workspace::Workspace;
use oxikube_workspace::test_support::{TestItem, open_workspace};

/// A user keymap of `n` distinct bindings in `Workspace` (in force wherever the bench focuses),
/// all of one action.
fn big_keymap(n: usize) -> String {
    let mut text = String::from(r#"[{"context": "Workspace", "bindings": {"#);
    let keys: Vec<String> = ('a'..='z')
        .map(String::from)
        .chain((0..10).map(|d| d.to_string()))
        .chain((1..=12).map(|f| format!("f{f}")))
        .collect();
    let leaders = [
        "ctrl-k",
        "ctrl-j",
        "ctrl-x",
        "ctrl-b",
        "alt-k",
        "alt-j",
        "alt-x",
        "alt-b",
        "cmd-k",
        "cmd-j",
        "cmd-x",
        "cmd-b",
        "ctrl-alt-k",
        "ctrl-alt-j",
        "ctrl-alt-x",
        "ctrl-alt-b",
        "ctrl-shift-k",
        "ctrl-shift-j",
        "ctrl-shift-x",
        "ctrl-shift-b",
        "alt-shift-k",
        "alt-shift-j",
        "alt-shift-x",
        "alt-shift-b",
        "cmd-shift-k",
        "cmd-shift-j",
        "cmd-shift-x",
        "cmd-shift-b",
        "cmd-alt-k",
        "cmd-alt-j",
        "cmd-alt-x",
        "cmd-alt-b",
        "ctrl-cmd-k",
        "ctrl-cmd-j",
        "ctrl-cmd-x",
        "ctrl-cmd-b",
        "ctrl-alt-shift-k",
        "ctrl-alt-shift-j",
        "ctrl-alt-shift-x",
        "ctrl-alt-shift-b",
    ];
    let mut count = 0;
    'outer: for leader in leaders {
        for key in &keys {
            if count == n {
                break 'outer;
            }
            if count > 0 {
                text.push_str(", ");
            }
            let _ = write!(text, r#""{leader} {key}": "workspace::SplitLeft""#);
            count += 1;
        }
    }
    text.push_str("}}]");
    text
}

struct Window {
    vcx: VisualTestContext,
    workspace: Entity<Workspace>,
    host: Rc<HelpHost>,
}

fn window(cx: &mut TestAppContext, user_keymap: &str) -> Window {
    let (workspace, mut vcx) = open_workspace(cx);
    vcx.update(|window, cx| {
        oxikube_keymap::init_with_text(user_keymap, KeymapOptions::default(), cx);
        let item = TestItem::build("Pods", cx);
        workspace.update(cx, |ws, cx| ws.open_item(item.clone(), window, cx));
        let focus: FocusHandle = item.read(cx).focus_handle(cx);
        focus.focus(window, cx);
    });
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let host = Rc::new(HelpHost::new(&workspace));
    Window {
        vcx,
        workspace,
        host,
    }
}

fn percentile(sorted: &[f64], p: usize) -> f64 {
    sorted[(sorted.len() * p / 100).min(sorted.len() - 1)]
}

fn report(name: &str, mut ms: Vec<f64>) {
    ms.sort_by(|a, b| a.total_cmp(b));
    let mean = ms.iter().sum::<f64>() / ms.len() as f64;
    println!(
        "help_bench {name}: {} runs, ms mean {mean:.3} p50 {:.3} p95 {:.3} max {:.3}",
        ms.len(),
        percentile(&ms, 50),
        percentile(&ms, 95),
        ms[ms.len() - 1]
    );
}

fn overlay_open(w: &Window) -> bool {
    let workspace = w.workspace.clone();
    w.vcx.clone().update(|_, cx| {
        workspace
            .read(cx)
            .modal_layer()
            .read(cx)
            .active_modal::<HelpOverlay>()
            .is_some()
    })
}

fn bench(label: &str, user_keymap: &str, entries: &str) {
    let mut cx = TestAppContext::single();
    let mut w = window(&mut cx, user_keymap);

    // Resolving the bindings of the focused view: once per open.
    let mut capture = Vec::new();
    let mut count = 0;
    for _ in 0..30 {
        let started = Instant::now();
        let model = w.vcx.update(|window, cx| HelpModel::capture(window, cx));
        capture.push(started.elapsed().as_secs_f64() * 1000.0);
        count = model.entries().len();
    }
    println!("help_bench {label}: {count} bindings in force ({entries})");
    report(&format!("{label} capture"), capture);

    // Opening: capture, the modal layer, the picker, the first frame; and closing.
    let mut open = Vec::new();
    for _ in 0..30 {
        let host = w.host.clone();
        let started = Instant::now();
        w.vcx.update(|window, cx| host.toggle(window, cx));
        w.vcx.run_until_parked();
        w.vcx.update(|window, cx| window.draw(cx).clear(cx));
        open.push(started.elapsed().as_secs_f64() * 1000.0);
        assert!(overlay_open(&w));
        let host = w.host.clone();
        w.vcx.update(|window, cx| host.toggle(window, cx));
        w.vcx.run_until_parked();
    }
    report(&format!("{label} open (to first frame)"), open);

    // A keystroke in the search field: the rows for the new query, then the frame.
    let host = w.host.clone();
    w.vcx.update(|window, cx| host.toggle(window, cx));
    w.vcx.run_until_parked();
    w.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let workspace = w.workspace.clone();
    let overlay: Entity<HelpOverlay> = w.vcx.update(|_, cx| {
        workspace
            .read(cx)
            .modal_layer()
            .read(cx)
            .active_modal()
            .expect("open")
    });
    let picker = w.vcx.update(|_, cx| overlay.read(cx).picker().clone());
    let queries = [
        "y",
        "yaml",
        "ctrl",
        "ctrl-k",
        "delete",
        "zzz",
        "",
        "cmd shift",
        "view",
        "a",
    ];
    let mut keystroke = Vec::new();
    let mut frame = Vec::new();
    for round in 0..60 {
        let query = queries[round % queries.len()];
        let picker = picker.clone();
        let started = Instant::now();
        w.vcx
            .update(|window, cx| picker.update(cx, |p, cx| p.set_query(query, window, cx)));
        w.vcx.run_until_parked();
        keystroke.push(started.elapsed().as_secs_f64() * 1000.0);
        let started = Instant::now();
        w.vcx.update(|window, cx| window.draw(cx).clear(cx));
        frame.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report(&format!("{label} keystroke (query to rows in)"), keystroke);
    report(&format!("{label} keystroke (frame after)"), frame);
}

fn main() {
    bench(
        "shipped keymap",
        "",
        "the actions the default keymaps name that this bench links",
    );
    bench("400-binding keymap", &big_keymap(400), "Workspace");
    bench("2000-binding keymap", &big_keymap(2_000), "Workspace");
}
