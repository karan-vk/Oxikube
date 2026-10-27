//! Large files and the validation pipeline (E10-S11): a 5 MB `kubectl get -o yaml` dump is
//! validated in chunks with the visible objects first and the gutter paints only the visible
//! markers; twenty quick keystrokes start one pass; an edit cancels the pass it outruns; and random
//! edits racing the pass never show diagnostics of an older buffer.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Entity, Focusable as _, SharedString, TestAppContext, VisualTestContext,
};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_editor::view::{
    LARGE_BUFFER_BYTES, ManifestEditor, ManifestEditorParts, PassStats, SchemaSource,
    VALIDATION_DEBOUNCE,
};
use oxikube_ports::SchemaPort;
use oxikube_testkit::fakes::FakeSchemaPort;
use oxikube_testkit::manifests::{FIVE_MB, faulty_count, pod_gvk, pod_list_yaml, pod_schema};
use oxikube_ui::editor::{EditorApi as _, OverlayStats};
use oxikube_ui::root::Root;
use oxikube_workspace::CommandDispatcher;

struct NoBus;

impl CommandDispatcher for NoBus {
    fn dispatch(&self, _: Command, _: &mut App) {}
}

struct Schemas {
    cluster: ClusterId,
    port: Arc<FakeSchemaPort>,
}

impl SchemaSource for Schemas {
    fn cluster(&self) -> &ClusterId {
        &self.cluster
    }
    fn label(&self, _: &App) -> SharedString {
        "test".into()
    }
    fn port(&self, _: &App) -> Option<Arc<dyn SchemaPort>> {
        Some(self.port.clone())
    }
}

/// A window holding a manifest editor with `text`, checked against a cluster serving the Pod
/// schema, with the keyboard focus in its buffer.
fn open(cx: &mut TestAppContext, text: String) -> (Entity<ManifestEditor>, VisualTestContext) {
    cx.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_runtime::init_deterministic(cx);
        oxikube_editor::init(cx);
    });
    let cluster = ClusterId::new("/kubeconfig", &ContextName::new("test"));
    let port = Arc::new(FakeSchemaPort::new());
    port.insert(cluster.clone(), pod_gvk(), pod_schema());
    let mut editor = None;
    let window = cx.add_window(|window, cx| {
        let parts = ManifestEditorParts {
            title: "dump.yaml".into(),
            text,
            schemas: Some(Rc::new(Schemas { cluster, port })),
            dispatcher: Rc::new(NoBus),
            resource: None,
            live: None,
        };
        let view = cx.new(|cx| ManifestEditor::new(parts, window, cx));
        editor = Some(view.clone());
        Root::new(view, window, cx)
    });
    let editor = editor.expect("built");
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.update(|window, cx| {
        window.activate_window();
        let buffer = editor.read(cx).focus_handle(cx);
        window.focus(&buffer, cx);
    });
    vcx.run_until_parked();
    (editor, vcx)
}

fn stats(vcx: &mut VisualTestContext, editor: &Entity<ManifestEditor>) -> PassStats {
    vcx.read(|cx| editor.read(cx).model().stats())
}

fn draw(vcx: &mut VisualTestContext) {
    vcx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
}

fn overlay(vcx: &mut VisualTestContext, editor: &Entity<ManifestEditor>) -> OverlayStats {
    vcx.read(|cx| editor.read(cx).editor().read(cx).overlay_stats())
}

/// The shown diagnostics belong to the buffer as it is: never a result of an older version.
fn assert_not_stale(vcx: &mut VisualTestContext, editor: &Entity<ManifestEditor>) {
    vcx.read(|cx| {
        let view = editor.read(cx);
        let code = view.editor().read(cx);
        let version = code.version();
        if let Some(shown) = view.model().shown_version() {
            assert_eq!(
                shown, version,
                "diagnostics of version {shown} shown at {version}"
            );
        }
        let problems = view.model().problems();
        if problems.errors + problems.warnings > 0 {
            assert_eq!(
                view.model().shown_version(),
                Some(version),
                "{} errors / {} warnings counted but diagnostics not shown for version {version}",
                problems.errors,
                problems.warnings
            );
        }
        let squiggles = code
            .state()
            .read(cx)
            .diagnostics()
            .map_or(0, |set| set.len());
        if view.model().shown_version().is_none() {
            assert_eq!(squiggles, 0, "an edit drops the squiggles");
        }
    });
}

#[gpui::test]
fn a_5mb_dump_is_validated_in_chunks_visible_first_and_the_gutter_paints_what_is_visible(
    cx: &mut TestAppContext,
) {
    // Every pod has a planted error: far more than the 1 000-diagnostic cap.
    let text = pod_list_yaml(FIVE_MB, Some(1));
    let len = text.len();
    let planted = faulty_count(&text);
    assert!(planted > 5_000);
    let (editor, mut vcx) = open(cx, text);
    // Opened: the first pass found a Pod in its first chunk and fetched the schema, whose
    // arrival cancelled it and started a pass that reused the parse; that one stopped at the cap.
    let s = stats(&mut vcx, &editor);
    assert_eq!((s.started, s.completed, s.cancelled), (2, 1, 1), "{s:?}");
    assert_eq!(
        s.parses, 1,
        "the schema's re-validation reused the parse: {s:?}"
    );
    assert!(s.chunks >= 3, "{s:?}");
    let (problems, coverage) = vcx.read(|cx| {
        let model = editor.read(cx).model();
        (model.problems(), model.coverage())
    });
    assert_eq!(problems.errors, 1_000, "the cap");
    assert!(coverage.is_complete());
    draw(&mut vcx);
    assert!(
        vcx.debug_bounds("manifest-large-file").is_some(),
        "the toolbar says the file is large"
    );

    // Scroll to the end and edit there: the next pass checks the objects on screen first, so
    // they have their diagnostics although the cap fills up with the start of the buffer.
    let code = vcx.read(|cx| editor.read(cx).editor().clone());
    vcx.update(|window, cx| code.update(cx, |code, cx| code.api(window, cx).select(len..len)));
    draw(&mut vcx);
    vcx.simulate_input(" ");
    vcx.executor().advance_clock(VALIDATION_DEBOUNCE);
    vcx.run_until_parked();
    draw(&mut vcx);
    let last_error = vcx.read(|cx| {
        let code = editor.read(cx).editor().read(cx);
        let set = code.state().read(cx).diagnostics().expect("a code editor");
        set.iter().map(|entry| entry.range.start).max().unwrap_or(0)
    });
    assert!(
        last_error > len - 2_000,
        "the visible end was checked first"
    );

    let shown = overlay(&mut vcx, &editor);
    let visible = vcx
        .read(|cx| code.read(cx).state().read(cx).visible_row_range())
        .expect("laid out");
    assert!(shown.lines >= 900, "{shown:?}");
    assert!(
        shown.painted > 0,
        "the visible faulty pods are marked: {shown:?}"
    );
    assert!(
        shown.painted <= visible.len(),
        "{} markers painted for {} visible lines",
        shown.painted,
        visible.len()
    );
    assert_not_stale(&mut vcx, &editor);
}

#[gpui::test]
fn twenty_quick_keystrokes_start_one_pass(cx: &mut TestAppContext) {
    let (editor, mut vcx) = open(cx, pod_list_yaml(8 * 1024, Some(10)));
    let before = stats(&mut vcx, &editor);
    assert!(before.completed >= 1, "validated on open: {before:?}");
    draw(&mut vcx);
    assert!(
        vcx.debug_bounds("manifest-large-file").is_none(),
        "a small file has no notice"
    );
    for _ in 0..20 {
        vcx.simulate_input("x");
        vcx.executor().advance_clock(VALIDATION_DEBOUNCE / 5);
    }
    assert_eq!(
        stats(&mut vcx, &editor).started,
        before.started,
        "still typing"
    );
    vcx.executor().advance_clock(VALIDATION_DEBOUNCE);
    vcx.run_until_parked();
    let after = stats(&mut vcx, &editor);
    assert_eq!(after.started, before.started + 1, "{after:?}");
    assert_eq!(after.completed, before.completed + 1, "{after:?}");
    assert_eq!(after.cancelled, before.cancelled);
    assert_not_stale(&mut vcx, &editor);
}

#[gpui::test]
fn an_edit_cancels_the_pass_it_outruns(cx: &mut TestAppContext) {
    let text = pod_list_yaml(LARGE_BUFFER_BYTES + 64 * 1024, Some(7));
    let (editor, mut vcx) = open(cx, text);
    let before = stats(&mut vcx, &editor);
    vcx.simulate_input("x");
    // Fire the debounce timer without running what it wakes (`advance_clock` would run the whole
    // pass), then run the pass a few steps: it takes the buffer but has chunks left.
    let dispatcher = vcx.executor().dispatcher().clone();
    let test = dispatcher.as_test().expect("the test dispatcher");
    for _ in 0..1_000 {
        if stats(&mut vcx, &editor).started > before.started {
            break;
        }
        if !vcx.executor().tick() {
            assert!(
                test.advance_clock_to_next_timer(),
                "the debounce timer is pending"
            );
        }
    }
    // The pass has taken the buffer; let it parse and check a chunk or two.
    for _ in 0..3 {
        vcx.executor().tick();
    }
    let running = stats(&mut vcx, &editor);
    assert_eq!(running.started, before.started + 1, "{running:?}");
    assert_eq!(running.completed, before.completed, "{running:?}");
    vcx.simulate_input("y");
    let cancelled = stats(&mut vcx, &editor);
    assert_eq!(cancelled.cancelled, before.cancelled + 1, "{cancelled:?}");
    assert_not_stale(&mut vcx, &editor);
    vcx.executor().advance_clock(VALIDATION_DEBOUNCE);
    vcx.run_until_parked();
    let done = stats(&mut vcx, &editor);
    assert_eq!(done.completed, before.completed + 1, "{done:?}");
    assert_not_stale(&mut vcx, &editor);
}

/// A small deterministic generator, so the stress test needs no `rand`.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, below: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % below
    }
}

#[gpui::test]
fn random_edits_racing_the_pass_never_show_stale_diagnostics(cx: &mut TestAppContext) {
    let text = pod_list_yaml(LARGE_BUFFER_BYTES + 32 * 1024, Some(5));
    let (editor, mut vcx) = open(cx, text);
    let dispatcher = vcx.executor().dispatcher().clone();
    let test = dispatcher.as_test().expect("the test dispatcher");
    for seed in [1_u64, 7, 42] {
        let mut rng = Lcg(seed);
        for _ in 0..40 {
            match rng.next(7) {
                0 => vcx.simulate_input("x"),
                1 => vcx.simulate_input("\n"),
                2 => vcx.simulate_keystrokes("backspace"),
                3 => vcx
                    .executor()
                    .advance_clock(Duration::from_millis(rng.next(300))),
                // Fire the next timer (the debounce, most often) without running what it wakes.
                4 => {
                    test.advance_clock_to_next_timer();
                }
                _ => {
                    for _ in 0..rng.next(8) {
                        vcx.executor().tick();
                    }
                }
            }
            assert_not_stale(&mut vcx, &editor);
        }
        vcx.executor().advance_clock(VALIDATION_DEBOUNCE);
        vcx.run_until_parked();
        assert_not_stale(&mut vcx, &editor);
        let (shown, version) = vcx.read(|cx| {
            let view = editor.read(cx);
            (
                view.model().shown_version(),
                view.editor().read(cx).version(),
            )
        });
        assert_eq!(shown, Some(version), "seed {seed}: the last pass is shown");
    }
}
