//! The `resource-diff` scenario (E20-S09): the cross-cluster resource diff tab over a 1 MB
//! ConfigMap (docs/PERFORMANCE.md "Editor": a diff of a large object must not stall the editor).
//!
//! The ConfigMap holds 64 files of 16 KiB (about 200 lines each, 12 800 lines in all). Two
//! comparisons are made of two copies of it:
//!
//! 1. **Compute** (`compute_ms`): what the `resource::DiffWith` handler does off the UI thread
//!    once both objects are read: normalise both sides, the changed-path summary, both written
//!    as YAML and the line diff (`DiffData::new`). Repeated, with the few changes of a real
//!    "is staging like prod" question (8 files differ by a line); `compute_worst_ms` is the same
//!    with every line different, which runs into the line diff's 500 ms timeout.
//! 2. **Repaint** (`frame_ms` / `draw_ms`, the budget's metrics): [`FRAMES`] frames of the tab
//!    over the few-changes diff, nothing changed.
//! 3. **Scroll** (`scroll_frame_ms` / `scroll_draw_ms`): [`FRAMES`] frames scrolling the
//!    every-line-different diff (25 000 side-by-side rows) from the top, 200 rows a frame; only
//!    the rows on screen are built.
//!
//! The sample fails when the view's text is drawn in a family the machine lacks (#509).

use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, Result, ensure};
use gpui::{AnyWindowHandle, App, AppContext as _, Entity};
use oxikube_app::diff::{DiffOptions, DiffRequest, IgnorePaths, compare};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_editor::view::{DiffData, DiffState, ResourceDiffView};
use oxikube_runtime::perf::harness::{self, FrameRun};
use oxikube_runtime::perf::{Recorder, ScenarioSample, Summary};
use oxikube_testkit::headless;
use oxikube_workspace::CommandDispatcher;
use serde_json::{Map, Value, json};

use super::{FRAMES, WINDOW_SIZE, window_root};

/// The scenario's name.
pub(super) const NAME: &str = "resource-diff";

/// Overrides the frames per phase (a debug build in the smoke test uses a few).
const FRAMES_ENV: &str = "OXIKUBE_PERF_RESOURCE_DIFF_FRAMES";

/// Overrides the number of files in the ConfigMap (64 files of 16 KiB make 1 MB).
const FILES_ENV: &str = "OXIKUBE_PERF_RESOURCE_DIFF_FILES";

/// How many times the few-changes diff is computed.
const COMPUTE_RUNS: usize = 5;

/// The lines of one file and their width: 16 KiB.
const LINES_PER_FILE: usize = 200;
const LINE_WIDTH: usize = 80;

struct NoBus;

impl CommandDispatcher for NoBus {
    fn dispatch(&self, _: Command, _: &mut App) {}
}

fn env_number(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// A file of [`LINES_PER_FILE`] lines; `salt` changes the text of every line when given, `edit`
/// changes the text of one.
fn file(index: usize, salt: Option<&str>, edit: bool) -> String {
    (0..LINES_PER_FILE)
        .map(|line| {
            let tag = match (salt, edit && line == 7) {
                (Some(salt), _) => salt,
                (None, true) => "edited",
                (None, false) => "same",
            };
            let text = format!("file {index} line {line} {tag} ");
            format!("{text:-<LINE_WIDTH$}\n")
        })
        .collect()
}

fn config_map(
    files: usize,
    namespace: &str,
    differing: impl Fn(usize) -> (Option<&'static str>, bool),
) -> Resource {
    let data: Map<String, Value> = (0..files)
        .map(|i| {
            let (salt, edit) = differing(i);
            (
                format!("file-{i:03}.conf"),
                Value::String(file(i, salt, edit)),
            )
        })
        .collect();
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "big", "namespace": namespace, "uid": "u", "resourceVersion": "5"},
        "data": data
    }))
    .expect("a ConfigMap")
}

fn reference(cluster: &str, namespace: &str) -> ResourceRef {
    let cluster = ClusterId::new("/kubeconfig", &ContextName::new(cluster));
    ResourceRef::namespaced(cluster, Gvk::new("", "v1", "ConfigMap"), namespace, "big")
}

fn options() -> DiffOptions {
    DiffOptions {
        ignore: IgnorePaths::standard(),
        ..DiffOptions::default()
    }
}

/// One sample. `probe` puts the `--perf` frame hook in the window.
pub(super) fn run(probe: bool) -> Result<ScenarioSample> {
    let recorder = Arc::new(Recorder::new());
    oxikube_runtime::perf::install(recorder.clone());
    let frames = env_number(FRAMES_ENV, FRAMES).clamp(1, FRAMES);
    let files = env_number(FILES_ENV, 64).max(1);
    let request = DiffRequest::new(
        reference("staging", "shop"),
        reference("production", "shop"),
    );

    // 1. Compute: what the handler does once both objects are read.
    let left = config_map(files, "shop", |_| (None, false));
    let right = config_map(files, "shop", |i| (None, i % 8 == 0));
    let bytes = serde_json::to_vec(&left.to_value())?.len();
    let mut compute_ns = Vec::new();
    let mut few = None;
    for _ in 0..COMPUTE_RUNS {
        let started = Instant::now();
        let diff = compare(
            &request,
            Some(left.clone()),
            Some(right.clone()),
            &options(),
        );
        let data = DiffData::new(diff);
        compute_ns.push(started.elapsed().as_nanos() as u64);
        few = Some(data);
    }
    let few = few.context("a diff")?;
    ensure!(few.lines.hunks() > 0, "the few-changes diff found nothing");
    let worst_right = config_map(files, "shop", |_| (Some("different"), false));
    let started = Instant::now();
    let worst = DiffData::new(compare(&request, Some(left), Some(worst_right), &options()));
    let worst_ns = vec![started.elapsed().as_nanos() as u64];
    let side_rows = worst.lines.side_rows().len();
    eprintln!(
        "oxikube resource-diff: ConfigMap of {} KiB, {} files; few changes: {} hunks; worst case: {side_rows} side rows",
        bytes / 1024,
        files,
        few.lines.hunks(),
    );

    // 2 and 3. The tab in the app's window root.
    let mut cx = headless::headless_context_with_assets(Arc::new(oxikube_ui::Assets));
    cx.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        oxikube_runtime::init_deterministic(cx);
    });
    let hook = probe.then(|| recorder.clone());
    let mut tab: Option<Entity<ResourceDiffView>> = None;
    let mut text_font = None;
    let window_handle: AnyWindowHandle = cx
        .open_window(WINDOW_SIZE, |window, cx| {
            let view = cx.new(|cx| {
                let mut view = ResourceDiffView::new(request.clone(), Rc::new(NoBus), cx);
                view.update_state(request.clone(), DiffState::Ready(Arc::new(few)), cx);
                view
            });
            tab = Some(view.clone());
            let (root, font) = window_root::mount(view.into(), hook, window, cx);
            text_font = Some(font);
            root
        })?
        .into();
    let tab = tab.context("the diff tab")?;
    harness::draw_frame(&mut cx, window_handle, |_, _| {})?;
    harness::draw_frame(&mut cx, window_handle, |_, _| {})?;
    cx.update(|cx| text_font.context("the window root")?.check(cx))?;

    let phase = |cx: &mut gpui::HeadlessAppContext,
                 step: &mut dyn FnMut(usize, &mut gpui::Window, &mut App)|
     -> Result<FrameRun> {
        let mut reader = recorder.reader();
        reader.drain(&recorder);
        harness::run_frames(
            cx,
            window_handle,
            frames,
            &recorder,
            &mut reader,
            |cx| cx.run_until_parked(),
            step,
        )
    };
    let main = phase(&mut cx, &mut |_, _, _| {})?;

    // The every-line-different diff, scrolled from the top.
    tab.update(&mut cx, |view, cx| {
        view.update_state(request.clone(), DiffState::Ready(Arc::new(worst)), cx);
    });
    harness::draw_frame(&mut cx, window_handle, |_, _| {})?;
    let mut scroll = phase(&mut cx, &mut |frame, _, cx| {
        tab.update(cx, |view, cx| view.scroll_to_row(frame * 200, cx));
    })?;

    let mut extra: Vec<(String, Summary)> = [
        (harness::metric::FRAME_MS, &mut scroll.frame_ns),
        (harness::metric::DRAW_MS, &mut scroll.draw_ns),
    ]
    .into_iter()
    .filter_map(|(name, samples)| {
        Summary::from_nanos(samples).map(|s| (format!("scroll_{name}"), s))
    })
    .collect();
    for (name, mut samples) in [("compute_ms", compute_ns), ("compute_worst_ms", worst_ns)] {
        extra.extend(Summary::from_nanos(&mut samples).map(|s| (name.to_owned(), s)));
    }
    Ok(main.into_sample(NAME, extra))
}
