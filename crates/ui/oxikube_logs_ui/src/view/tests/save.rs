//! Saving a log view's lines to a file (E08-S06): the dialog, the platform's save panel, the
//! write through the `FsPort`, filters, truncation, errors, and that nothing blocks the UI.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use futures::channel::oneshot;
use futures::stream::BoxStream;
use gpui::{Entity, TestAppContext};
use oxikube_app::logs::LogEntry;
use oxikube_app::logs::export::LineFilter;
use oxikube_domain::OxiError;
use oxikube_domain::OxiResult;
use oxikube_domain::command::Command;
use oxikube_domain::log::LogSaveScope;
use oxikube_ports::{DirEntry, FileChunks, FsEvent, FsPort};
use oxikube_testkit::{FakeFsPort, Timeline};
use oxikube_workspace::Workspace;
use parking_lot::Mutex;

use super::fixture::{Fx, line, lines, pod_ref};
use super::scroll::{tick, trickle};
use crate::export::SaveDialog;

const PATH: &str = "/exports/web-0.log";

fn open(fx: &mut Fx, n: usize) -> Entity<crate::LogView> {
    fx.open(Timeline::immediate(lines(0, n)).keep_open())
}

impl Fx {
    fn dialog(&mut self) -> Option<Entity<SaveDialog>> {
        let workspace = self.workspace.clone();
        self.vcx.update(|_, cx| {
            workspace
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<SaveDialog>()
        })
    }

    fn summary(&mut self) -> String {
        let dialog = self.dialog().expect("the save dialog is open");
        self.vcx.update(|_, cx| dialog.read(cx).summary())
    }

    /// Chooses the file in the platform's panel: `path`, or cancels with `None`.
    fn choose_file(&mut self, path: Option<&str>) {
        self.click("log-save-choose");
        let path = path.map(PathBuf::from);
        self.vcx.simulate_new_path_selection(move |_| path);
        self.settle();
    }

    fn written(&self, path: &str) -> Option<String> {
        self.fs
            .file(path)
            .map(|bytes| String::from_utf8(bytes).unwrap())
    }
}

fn expected(range: std::ops::Range<usize>) -> String {
    range.map(|i| format!("{}\n", line(i).text)).collect()
}

#[gpui::test]
fn ctrl_s_says_what_would_be_written_and_writes_it_to_the_chosen_file(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    open(&mut fx, 5);
    fx.keys("ctrl-s");
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::LogsSave {
            target: pod_ref(),
            scope: LogSaveScope::All
        })
    );
    assert!(fx.drawn("log-save-dialog"));
    assert_eq!(fx.summary(), "Everything the buffer holds: 5 lines.");
    assert!(
        fx.written(PATH).is_none(),
        "nothing is written before a file is chosen"
    );

    fx.choose_file(Some(PATH));
    assert_eq!(
        fx.written(PATH),
        Some(expected(0..5)),
        "the raw text of every line"
    );
    assert!(!fx.drawn("log-save-dialog"), "the dialog closed");
    assert!(
        fx.toasts()
            .iter()
            .any(|t| t == "Saved 5 lines to /exports/web-0.log"),
        "{:?}",
        fx.toasts()
    );
}

#[gpui::test]
fn visible_writes_the_lines_on_screen_and_all_the_whole_buffer(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx, 5_000);
    fx.draw();
    let seqs = fx
        .read(&view, |v| v.viewport_seqs())
        .expect("lines on screen");
    let on_screen = (seqs.end - seqs.start) as usize;
    assert!(
        on_screen > 5 && on_screen < 200,
        "{on_screen} rows on screen"
    );

    fx.keys("ctrl-shift-s");
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::LogsSave {
            target: pod_ref(),
            scope: LogSaveScope::Visible
        })
    );
    assert_eq!(
        fx.summary(),
        format!("The lines on screen: {on_screen} lines.")
    );
    fx.choose_file(Some("/exports/visible.log"));
    assert_eq!(
        fx.written("/exports/visible.log"),
        Some(expected(seqs.start as usize..seqs.end as usize))
    );

    fx.keys("ctrl-s");
    assert_eq!(fx.summary(), "Everything the buffer holds: 5,000 lines.");
    fx.choose_file(Some("/exports/all.log"));
    assert_eq!(fx.written("/exports/all.log"), Some(expected(0..5_000)));

    // The dialog's own buttons switch between the two.
    fx.keys("ctrl-s");
    fx.click("log-save-scope-visible");
    assert_eq!(
        fx.summary(),
        format!("The lines on screen: {on_screen} lines.")
    );
    fx.click("log-save-scope-all");
    assert_eq!(fx.summary(), "Everything the buffer holds: 5,000 lines.");
}

#[gpui::test]
fn the_timestamp_and_prefix_toggles_shape_the_lines(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    open(&mut fx, 3);
    fx.keys("ctrl-s");
    fx.click("log-save-timestamps");
    fx.click("log-save-prefix");
    fx.choose_file(Some(PATH));
    let text = fx.written(PATH).unwrap();
    let first = text.lines().next().unwrap();
    assert!(
        first.starts_with("2026-10-04T") && first.ends_with(" web-0/app INFO line 0"),
        "{first}"
    );

    // The default matches the screen: timestamps on screen are timestamps in the file.
    fx.keys("t");
    fx.keys("ctrl-s");
    fx.choose_file(Some("/exports/two.log"));
    let text = fx.written("/exports/two.log").unwrap();
    assert!(
        text.starts_with("2026-10-04T") && !text.contains("web-0/app"),
        "{text}"
    );
}

#[gpui::test]
fn the_filter_decides_what_is_counted_and_written(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx, 10);
    let even: LineFilter = Arc::new(|e: &LogEntry| e.seq.is_multiple_of(2));
    fx.vcx
        .update(|_, cx| view.update(cx, |v, cx| v.set_line_filter(Some(even), cx)));
    fx.keys("ctrl-s");
    fx.settle(); // the filtered count arrives off the UI thread
    assert_eq!(
        fx.summary(),
        "Everything the buffer holds: 5 lines, matching the filter."
    );
    fx.choose_file(Some(PATH));
    let written = fx.written(PATH).unwrap();
    let want: String = [0, 2, 4, 6, 8]
        .iter()
        .map(|i| format!("{}\n", line(*i).text))
        .collect();
    assert_eq!(written, want, "what you see is what you export");
}

#[gpui::test]
fn a_truncated_buffer_is_said_so_in_the_dialog(cx: &mut TestAppContext) {
    let mut fx = Fx::with_buffer(cx, 100);
    open(&mut fx, 250);
    fx.keys("ctrl-s");
    let dialog = fx.dialog().unwrap();
    let note = fx
        .vcx
        .update(|_, cx| dialog.read(cx).note().map(str::to_owned));
    let note = note.expect("the buffer dropped lines");
    assert!(
        note.contains("last 100 lines") && note.contains("150 older lines"),
        "{note}"
    );
    assert_eq!(fx.summary(), "Everything the buffer holds: 100 lines.");
    // A buffer that dropped nothing has no note.
    let mut other = Fx::new(cx);
    open(&mut other, 5);
    other.keys("ctrl-s");
    let dialog = other.dialog().unwrap();
    assert_eq!(
        other
            .vcx
            .update(|_, cx| dialog.read(cx).note().map(str::to_owned)),
        None
    );
}

#[gpui::test]
fn the_dialog_does_not_warn_about_lines_dropped_before_a_clear(cx: &mut TestAppContext) {
    let mut fx = Fx::with_buffer(cx, 100);
    let view = fx.open(trickle(100, 40));
    tick(&mut fx, 5); // seqs 0..5 leave the ring: the view shows its truncated marker
    assert!(fx.read(&view, |v| v.line_window().is_truncated()));
    fx.keys("shift-c");
    tick(&mut fx, 3);
    fx.read(&view, |v| {
        assert!(!v.line_window().is_truncated(), "the view shows no marker");
    });
    fx.keys("ctrl-s");
    let dialog = fx.dialog().unwrap();
    let note = fx
        .vcx
        .update(|_, cx| dialog.read(cx).note().map(str::to_owned));
    assert_eq!(note, None, "so the dialog has no note either");
    assert_eq!(fx.summary(), "Everything the buffer holds: 3 lines.");
}

#[gpui::test]
fn cancelling_the_dialog_or_the_panel_writes_nothing(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    open(&mut fx, 5);
    fx.keys("ctrl-s");
    fx.click("log-save-cancel");
    assert!(!fx.drawn("log-save-dialog"));

    fx.keys("ctrl-s");
    fx.choose_file(None); // the user closes the save panel
    assert!(
        fx.fs.recorded_calls().is_empty(),
        "the port was never asked"
    );
    assert!(fx.toasts().is_empty(), "{:?}", fx.toasts());
}

#[gpui::test]
fn an_error_from_the_fs_port_shows_a_message(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    open(&mut fx, 5);
    fx.fs
        .script()
        .write
        .push_err(OxiError::forbidden("no permission to write /ro/web-0.log"));
    fx.keys("ctrl-s");
    fx.choose_file(Some("/ro/web-0.log"));
    assert!(fx.written("/ro/web-0.log").is_none());
    assert!(
        fx.toasts()
            .iter()
            .any(|t| t == "Could not save the log: no permission to write /ro/web-0.log"),
        "{:?}",
        fx.toasts()
    );
}

/// An `FsPort` whose `write_stream` waits for a gate before it reads the first chunk.
struct GatedFs {
    inner: Arc<FakeFsPort>,
    gate: Mutex<Option<oneshot::Receiver<()>>>,
}

#[async_trait]
impl FsPort for GatedFs {
    async fn read(&self, path: &Path) -> OxiResult<Vec<u8>> {
        self.inner.read(path).await
    }
    async fn write(&self, path: &Path, contents: &[u8]) -> OxiResult<()> {
        self.inner.write(path, contents).await
    }
    async fn write_stream(&self, path: &Path, chunks: FileChunks) -> OxiResult<()> {
        let gate = self.gate.lock().take();
        if let Some(gate) = gate {
            let _ = gate.await;
        }
        self.inner.write_stream(path, chunks).await
    }
    async fn write_private(&self, path: &Path, contents: &[u8]) -> OxiResult<()> {
        self.inner.write_private(path, contents).await
    }
    async fn remove(&self, path: &Path) -> OxiResult<bool> {
        FsPort::remove(&*self.inner, path).await
    }
    async fn list(&self, path: &Path) -> OxiResult<Vec<DirEntry>> {
        self.inner.list(path).await
    }
    fn watch(&self, path: &Path) -> BoxStream<'static, FsEvent> {
        self.inner.watch(path)
    }
}

#[gpui::test]
fn a_big_export_does_not_block_the_ui(cx: &mut TestAppContext) {
    let (release, gate) = oneshot::channel();
    let mut fx = Fx::with_fs(cx, move |inner| {
        Arc::new(GatedFs {
            inner,
            gate: Mutex::new(Some(gate)),
        })
    });
    let view = open(&mut fx, 20_000);
    fx.keys("ctrl-s");
    fx.choose_file(Some("/exports/big.log"));

    // The write has not finished (the port holds it back) and the call came back anyway: the
    // view still takes keys, draws, and says it is saving.
    assert!(fx.written("/exports/big.log").is_none());
    fx.keys("w");
    assert!(
        fx.read(&view, |v| v.options().wrap),
        "the view still answers"
    );
    assert!(
        fx.toasts().iter().any(|t| t == "Saving big.log…"),
        "{:?}",
        fx.toasts()
    );

    release.send(()).unwrap();
    fx.settle();
    let written = fx.written("/exports/big.log").expect("the write finished");
    assert_eq!(written.lines().count(), 20_000);
    assert!(
        fx.toasts()
            .iter()
            .any(|t| t == "Saved 20,000 lines to /exports/big.log"),
        "{:?}",
        fx.toasts()
    );
}

/// A view with a 20 000 line buffer whose first write is held back by the port.
fn held_back(cx: &mut TestAppContext) -> (Fx, Entity<crate::LogView>, oneshot::Sender<()>) {
    let (release, gate) = oneshot::channel();
    let mut fx = Fx::with_fs(cx, move |inner| {
        Arc::new(GatedFs {
            inner,
            gate: Mutex::new(Some(gate)),
        })
    });
    let view = open(&mut fx, 20_000);
    fx.keys("ctrl-s");
    fx.choose_file(Some("/exports/a.log"));
    assert!(fx.toasts().iter().any(|t| t == "Saving a.log…"));
    (fx, view, release)
}

#[gpui::test]
fn closing_the_tab_mid_save_does_not_leave_a_saving_toast(cx: &mut TestAppContext) {
    let (mut fx, view, release) = held_back(cx);
    let workspace: Entity<Workspace> = fx.workspace.clone();
    let id = view.entity_id();
    fx.vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.close_item(id, window, cx);
        })
    });
    fx.settle();
    release.send(()).ok();
    fx.settle();
    assert_eq!(
        fx.toasts(),
        ["Stopped saving a.log."],
        "the progress toast says it stopped"
    );
    assert!(
        fx.written("/exports/a.log").is_none(),
        "the write was aborted"
    );
}

#[gpui::test]
fn a_second_save_stops_the_first_and_says_so(cx: &mut TestAppContext) {
    let (mut fx, _view, _release) = held_back(cx);
    fx.keys("ctrl-s");
    fx.choose_file(Some("/exports/b.log"));
    assert!(fx.written("/exports/a.log").is_none());
    assert_eq!(
        fx.written("/exports/b.log").map(|t| t.lines().count()),
        Some(20_000)
    );
    let toasts = fx.toasts();
    assert!(
        toasts.iter().any(|t| t == "Stopped saving a.log."),
        "{toasts:?}"
    );
    assert!(
        toasts
            .iter()
            .any(|t| t.starts_with("Saved 20,000 lines to /exports/b.log"))
    );
    assert!(
        !toasts.iter().any(|t| t.starts_with("Saving")),
        "{toasts:?}"
    );
}

#[gpui::test]
fn cancelling_a_second_panel_leaves_the_first_save_running(cx: &mut TestAppContext) {
    let (mut fx, _view, release) = held_back(cx);
    fx.keys("ctrl-s");
    fx.choose_file(None);
    assert!(
        fx.toasts().iter().any(|t| t == "Saving a.log…"),
        "{:?}",
        fx.toasts()
    );
    release.send(()).unwrap();
    fx.settle();
    assert_eq!(
        fx.written("/exports/a.log").map(|t| t.lines().count()),
        Some(20_000)
    );
    assert!(
        fx.toasts()
            .iter()
            .any(|t| t == "Saved 20,000 lines to /exports/a.log"),
        "{:?}",
        fx.toasts()
    );
}
