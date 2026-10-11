//! The `editor-5mb` scenario (E10-S11): the manifest editor over a 5 MB `kubectl get -o yaml`
//! dump (docs/PERFORMANCE.md "Editor": typing latency <= 16 ms with validation debounced, a 5 MB
//! file opens in <= 500 ms; the epic: frames over 16 ms < 1 %).
//!
//! It runs the real [`ManifestEditor`] over `oxikube_testkit::manifests::pod_list_yaml` (a
//! `kind: List` of pods, a schema error planted in every 20th) checked against the Pod schema
//! served by a fake `SchemaPort`, in a headless window built as the app builds its own
//! ([`window_root`]).
//!
//! 1. **Open** (`open_ms`): building the view with the 5 MB text (the rope, the highlighter) to the
//!    end of the first frame that shows it. Then the first validation settles (unmeasured).
//! 2. **Typing** (`frame_ms` / `draw_ms`): [`FRAMES`] scripted frames, each dispatching one
//!    keystroke into a visible line and drawing the frame that shows it; the clock moves one
//!    120 Hz frame between them, so the 150 ms validation debounce keeps restarting and no pass
//!    runs while typing, as in the app.
//! 3. **Validation** (`validation_ms`): the pause after typing; the whole pass the debounce
//!    starts (parse, the visible objects, then every chunk, each publish), run on the test
//!    scheduler, so it is the pass's CPU time, which the app spends on a background thread.
//! 4. **Scrolling** (`scroll_frame_ms` / `scroll_draw_ms`): [`FRAMES`] frames, each showing a
//!    screenful further into the dump (from the top to the end), the diagnostics drawn.
//!
//! `slow_frame_pct` is the share of the typing and scrolling frames over 16 ms. The sample fails
//! when the validation misses a planted error, when the gutter paints more markers than lines on
//! screen, or when the view's text is drawn in a family the machine lacks (#509).

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, ensure};
use gpui::{
    AnyWindowHandle, App, AppContext as _, Entity, Focusable as _, Keystroke, SharedString, point,
    px,
};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_editor::view::{
    ManifestEditor, ManifestEditorParts, SchemaSource, VALIDATION_DEBOUNCE,
};
use oxikube_ports::SchemaPort;
use oxikube_runtime::perf::harness::{self, metric};
use oxikube_runtime::perf::{Recorder, ScenarioSample, Summary, round_ms};
use oxikube_testkit::fakes::FakeSchemaPort;
use oxikube_testkit::headless;
use oxikube_testkit::manifests::{FIVE_MB, faulty_count, pod_gvk, pod_list_yaml, pod_schema};
use oxikube_ui::editor::EditorApi as _;
use oxikube_workspace::CommandDispatcher;

use super::{FRAMES, WINDOW_SIZE, window_root};

/// The scenario's name.
pub(super) const NAME: &str = "editor-5mb";

/// Overrides the dump's size in bytes ([`FIVE_MB`]): the smoke test runs a debug build on a
/// small one.
const BYTES_ENV: &str = "OXIKUBE_PERF_EDITOR_BYTES";

/// One pod in this many has a planted schema error: about 450 in 5 MB, under the 1 000 cap.
const FAULTY_EVERY: usize = 20;

/// The clock between two typing frames: one 120 Hz refresh.
const FRAME: Duration = Duration::from_micros(8_333);

/// A frame over this is a dropped 60 Hz frame (the epic's "frames over 16 ms").
const SLOW_FRAME_NS: u64 = 16_000_000;

struct NoBus;

impl CommandDispatcher for NoBus {
    fn dispatch(&self, _: Command, _: &mut App) {}
}

/// The cluster's schemas: a fake port serving the Pod schema.
struct Schemas {
    cluster: ClusterId,
    port: Arc<FakeSchemaPort>,
}

impl SchemaSource for Schemas {
    fn cluster(&self) -> &ClusterId {
        &self.cluster
    }
    fn label(&self, _: &App) -> SharedString {
        "perf".into()
    }
    fn port(&self, _: &App) -> Option<Arc<dyn SchemaPort>> {
        Some(self.port.clone())
    }
}

fn bytes() -> usize {
    std::env::var(BYTES_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(FIVE_MB)
}

/// One sample. `probe` puts the `--perf` frame hook in the window.
pub(super) fn run(probe: bool) -> Result<ScenarioSample> {
    let recorder = Arc::new(Recorder::new());
    oxikube_runtime::perf::install(recorder.clone());
    let text = pod_list_yaml(bytes(), Some(FAULTY_EVERY));
    let (len, planted, lines) = (text.len(), faulty_count(&text), text.lines().count());
    let cursor = text.find("name: web-000002").context("the third pod")? + 16;

    let mut cx = headless::headless_context_with_assets(Arc::new(oxikube_ui::Assets));
    cx.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        oxikube_runtime::init_deterministic(cx);
        oxikube_editor::init(cx);
    });
    let cluster = ClusterId::new("/perf/kubeconfig", &ContextName::new("perf"));
    let port = Arc::new(FakeSchemaPort::new());
    port.insert(cluster.clone(), pod_gvk(), pod_schema());

    // 1. Open.
    let hook = probe.then(|| recorder.clone());
    let started = Instant::now();
    let mut editor: Option<Entity<ManifestEditor>> = None;
    let mut text_font = None;
    let window: AnyWindowHandle = cx
        .open_window(WINDOW_SIZE, |window, cx| {
            let parts = ManifestEditorParts {
                title: "pods.yaml".into(),
                text,
                schemas: Some(Rc::new(Schemas { cluster, port })),
                dispatcher: Rc::new(NoBus),
                resource: None,
                live: None,
            };
            let view = cx.new(|cx| ManifestEditor::new(parts, window, cx));
            editor = Some(view.clone());
            let (root, font) = window_root::mount(view.into(), hook, window, cx);
            text_font = Some(font);
            root
        })?
        .into();
    harness::draw_frame(&mut cx, window, |_, _| {})?;
    let open = started.elapsed();
    let editor = editor.context("the editor view")?;
    cx.update(|cx| text_font.context("the window root")?.check(cx))?;
    cx.run_until_parked();
    let code = cx.update(|cx| editor.read(cx).editor().clone());
    let state = cx.update(|cx| code.read(cx).state().clone());

    // 2. Typing, one keystroke a frame into the third pod's name.
    harness::draw_frame(&mut cx, window, |window, cx| {
        window.focus(&editor.focus_handle(cx), cx);
        code.update(cx, |code, cx| code.api(window, cx).select(cursor..cursor));
    })?;
    let mut reader = recorder.reader();
    reader.drain(&recorder);
    let key = Keystroke::parse("x")?;
    let typing = harness::run_frames(
        &mut cx,
        window,
        FRAMES,
        &recorder,
        &mut reader,
        |cx| {
            cx.advance_clock(FRAME);
            cx.run_until_parked();
        },
        |_, window, cx| {
            window.dispatch_keystroke(key.clone(), cx);
        },
    )?;
    let typed = cx.update(|cx| code.read(cx).len(cx)) - len;
    ensure!(
        typed == FRAMES,
        "{typed} of {FRAMES} keystrokes reached the buffer"
    );

    // 3. The pause: one whole pass over the edited dump.
    let before = cx.update(|cx| editor.read(cx).model().stats());
    let paused = Instant::now();
    cx.advance_clock(VALIDATION_DEBOUNCE);
    cx.run_until_parked();
    let validation = paused.elapsed();
    let (after, problems) = cx.update(|cx| {
        let model = editor.read(cx).model();
        (model.stats(), model.problems())
    });
    ensure!(
        after.completed == before.completed + 1,
        "the pause ran one pass: {before:?} -> {after:?}"
    );
    ensure!(
        problems.errors == planted,
        "{} errors shown for {planted} planted",
        problems.errors
    );

    // 4. Scrolling from the top to the end, a jump a frame.
    let line_height = cx
        .update(|cx| state.read(cx).line_height())
        .context("laid out")?;
    let mut painted = 0;
    let mut scrolling = harness::run_frames(
        &mut cx,
        window,
        FRAMES,
        &recorder,
        &mut reader,
        |cx| cx.run_until_parked(),
        |frame, _, cx| {
            painted = painted.max(code.read(cx).overlay_stats().painted);
            let y = line_height * (lines * (frame + 1) / FRAMES) as f32;
            state.update(cx, |state, cx| {
                state.set_scroll_offset(point(px(0.), -y), cx)
            });
        },
    )?;
    let shown = cx.update(|cx| code.read(cx).overlay_stats());
    let visible = cx
        .update(|cx| state.read(cx).visible_row_range())
        .map_or(0, |rows| rows.len());
    ensure!(
        painted.max(shown.painted) <= visible,
        "the gutter painted {} markers for {visible} visible lines",
        painted.max(shown.painted)
    );

    let slow = typing
        .draw_ns
        .iter()
        .chain(&scrolling.draw_ns)
        .filter(|&&ns| ns > SLOW_FRAME_NS)
        .count();
    let frames = typing.draw_ns.len() + scrolling.draw_ns.len();
    let open_ms = round_ms(open.as_secs_f64() * 1000.0);
    let validation_ms = round_ms(validation.as_secs_f64() * 1000.0);
    eprintln!(
        "oxikube editor-5mb: opened {len} bytes in {open_ms} ms; {typed} keystrokes; one pass in \
         {validation_ms} ms ({} errors); {slow} of {frames} frames over 16 ms",
        problems.errors
    );
    let mut extra = vec![
        ("open_ms".to_owned(), Summary::single(open_ms)),
        ("validation_ms".to_owned(), Summary::single(validation_ms)),
        (
            "slow_frame_pct".to_owned(),
            Summary::single(slow as f64 * 100.0 / frames.max(1) as f64),
        ),
    ];
    for (name, samples) in [
        (metric::FRAME_MS, &mut scrolling.frame_ns),
        (metric::DRAW_MS, &mut scrolling.draw_ns),
    ] {
        if let Some(summary) = Summary::from_nanos(samples) {
            extra.push((format!("scroll_{name}"), summary));
        }
    }
    Ok(typing.into_sample(NAME, extra))
}
