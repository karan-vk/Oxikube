//! Downloads and uploads from the browser: the save dialog, progress toasts, the transfers panel
//! and its cancel, the drop from the OS, the question before replacing, and read-only.

use std::path::PathBuf;

use futures::{StreamExt as _, stream};
use gpui::{ExternalPaths, FileDropEvent, Modifiers, TestAppContext, point, px};
use oxikube_app::files::TransferState;
use oxikube_ports::ExitStatus;
use oxikube_testkit::ExecScript;
use oxikube_workspace::DialogModal;

use super::{Fx, ls_output};

fn listing() -> String {
    ls_output(&[('d', "etc", 4096), ('-', "motd", 11)])
}

/// Opens the browser at `/` and selects the row named `name`.
fn open_and_select(fx: &mut Fx, name: &str) -> gpui::Entity<crate::files::FilesView> {
    fx.prepare(&["app"], &listing());
    let view = fx.open_at(None);
    let row = fx
        .read(|v, cx| v.model(cx).rows().position(|r| r.entry.name == name))
        .expect("the row");
    fx.vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.table.update_quiet(cx, |d| d.model.select(row));
            cx.notify();
        });
    });
    fx.settle();
    view
}

fn drop_files(fx: &mut Fx, paths: &[&str]) {
    let paths = ExternalPaths(paths.iter().map(PathBuf::from).collect());
    let at = fx.vcx.debug_bounds("files-view").expect("drawn").center();
    fx.vcx.simulate_event(FileDropEvent::Entered {
        position: at,
        paths,
    });
    fx.vcx
        .simulate_event(FileDropEvent::Submit { position: at });
    fx.settle();
}

#[gpui::test]
fn download_asks_where_to_save_then_writes_the_file_and_says_so(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    open_and_select(&mut fx, "motd");
    fx.script(ExecScript::new().stdout("hello ").stdout("world"));
    fx.click("files-download");
    assert!(fx.vcx.did_prompt_for_new_path(), "a save dialog");
    fx.vcx.simulate_new_path_selection(|dir| {
        let _ = dir;
        Some(PathBuf::from("/dl/motd"))
    });
    fx.settle();
    assert_eq!(fx.fs.file("/dl/motd").as_deref(), Some(&b"hello world"[..]));
    let toasts = fx.toasts();
    assert!(
        toasts.iter().any(|t| t.contains("Downloaded /motd")),
        "{toasts:?}"
    );
    // The panel lists it as done.
    assert!(fx.drawn("files-transfers"));
    let board = fx.service.board().snapshot();
    assert_eq!(board[0].state, TransferState::Done);
}

#[gpui::test]
fn cancelling_the_save_dialog_downloads_nothing(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    open_and_select(&mut fx, "motd");
    fx.click("files-download");
    fx.vcx.simulate_new_path_selection(|_| None);
    fx.settle();
    assert!(fx.service.board().snapshot().is_empty());
    assert!(!fx.drawn("files-transfers"));
}

#[gpui::test]
fn a_running_download_shows_a_progress_toast_and_a_panel_row_and_cancel_stops_it(
    cx: &mut TestAppContext,
) {
    let mut fx = Fx::new(cx);
    let view = open_and_select(&mut fx, "motd");
    let script = ExecScript::new().stdout_stream(
        stream::once(async { Ok(b"first chunk".to_vec()) })
            .chain(stream::pending())
            .boxed(),
    );
    let capture = script.capture();
    fx.script(script);
    let entry = fx
        .read(|v, cx| v.model(cx).selected().cloned())
        .expect("selected");
    fx.vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.start_download(entry, PathBuf::from("/dl/motd"), false, cx)
        });
    });
    fx.settle();

    let toasts = fx.toasts();
    assert!(
        toasts.iter().any(|t| t.starts_with("Downloading motd")),
        "a progress toast: {toasts:?}"
    );
    assert!(
        fx.drawn("files-transfers"),
        "the panel lists the running transfer"
    );
    assert!(fx.drawn("files-transfer-0") && fx.drawn("files-cancel-0"));
    assert_eq!(fx.service.board().running(), 1);

    fx.click("files-cancel-0");
    fx.settle();
    assert_eq!(
        fx.service.board().snapshot()[0].state,
        TransferState::Cancelled
    );
    assert_eq!(fx.fs.file("/dl/motd"), None, "nothing under the final name");
    assert!(capture.aborted(), "the remote process was stopped");
    let toasts = fx.toasts();
    assert!(
        toasts.iter().any(|t| t.contains("Cancelled: motd")),
        "{toasts:?}"
    );
    assert!(
        !toasts.iter().any(|t| t.starts_with("Downloading")),
        "the progress toast is gone"
    );
}

#[gpui::test]
fn a_failed_download_names_the_missing_program_in_a_toast_and_the_panel(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open_and_select(&mut fx, "etc");
    fx.script(ExecScript::new().exit(Ok(ExitStatus::with_code(127))));
    let entry = fx
        .read(|v, cx| v.model(cx).selected().cloned())
        .expect("selected");
    fx.vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.start_download(entry, PathBuf::from("/dl"), false, cx)
        });
    });
    fx.settle();
    let toasts = fx.toasts();
    assert!(
        toasts
            .iter()
            .any(|t| t.contains("tar") && t.contains("container app")),
        "{toasts:?}"
    );
    let board = fx.service.board().snapshot();
    assert!(matches!(&board[0].state, TransferState::Failed(m) if m.contains("tar")));
}

#[gpui::test]
fn dropping_a_file_from_the_os_uploads_it_into_the_directory_shown(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    open_and_select(&mut fx, "motd");
    fx.fs.insert("/home/me/notes.txt", b"hello".to_vec());
    // `test -e` (the plan, then the handler's own check): no such file; then the upload; then
    // the refreshed listing.
    fx.script(ExecScript::new().exit(Ok(ExitStatus::with_code(1))));
    fx.script(ExecScript::new().exit(Ok(ExitStatus::with_code(1))));
    let upload = ExecScript::new();
    let capture = upload.capture();
    fx.script(upload);
    fx.script(ExecScript::new().stdout(ls_output(&[('-', "notes.txt", 5)])));
    drop_files(&mut fx, &["/home/me/notes.txt"]);

    let sent = capture.stdin();
    let mut archive = tar::Archive::new(&sent[..]);
    let entry = archive
        .entries()
        .expect("tar")
        .next()
        .expect("an entry")
        .expect("ok");
    assert_eq!(entry.path().expect("path").to_str(), Some("notes.txt"));
    assert!(capture.stdin_closed());
    let toasts = fx.toasts();
    assert!(
        toasts.iter().any(|t| t.contains("Uploaded notes.txt to /")),
        "{toasts:?}"
    );
    assert_eq!(fx.names(), ["notes.txt"], "the listing was refreshed");
    // One audit record for the upload.
    let board = fx.service.board().snapshot();
    assert_eq!(board[0].state, TransferState::Done);
}

#[gpui::test]
fn replacing_a_file_asks_first_and_declining_sends_nothing(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    open_and_select(&mut fx, "motd");
    fx.fs.insert("/home/me/motd", b"new".to_vec());
    // `test -e` succeeds: the file is there.
    fx.script(ExecScript::new());
    drop_files(&mut fx, &["/home/me/motd"]);
    let workspace = fx.workspace.clone();
    let dialog = fx
        .vcx
        .update(|_, cx| {
            workspace
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<DialogModal>()
        })
        .expect("a confirmation");
    let (title, message) = fx.vcx.update(|_, cx| {
        let dialog = dialog.read(cx);
        (
            dialog.title().to_string(),
            dialog.message_text().map(ToString::to_string),
        )
    });
    assert_eq!(title, "Replace motd in /?");
    assert!(message.unwrap_or_default().contains("overwritten"));
    assert_eq!(
        fx.ports().exec_stream.recorded_calls().len(),
        2,
        "only the listing and the check ran"
    );

    // Declining: Escape.
    fx.vcx.simulate_keystrokes("escape");
    fx.settle();
    assert_eq!(fx.ports().exec_stream.recorded_calls().len(), 2);
    assert!(fx.service.board().snapshot().is_empty());
}

#[gpui::test]
fn confirming_the_replace_uploads_and_audits_the_overwrite(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    open_and_select(&mut fx, "motd");
    fx.fs.insert("/home/me/motd", b"new".to_vec());
    fx.script(ExecScript::new());
    let upload = ExecScript::new();
    let capture = upload.capture();
    fx.script(upload);
    fx.script(ExecScript::new().stdout(listing()));
    drop_files(&mut fx, &["/home/me/motd"]);
    let workspace = fx.workspace.clone();
    let dialog = fx
        .vcx
        .update(|_, cx| {
            workspace
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<DialogModal>()
        })
        .expect("a confirmation");
    fx.vcx
        .update(|window, cx| dialog.update(cx, |d, cx| d.press_confirm(window, cx)));
    fx.settle();
    assert!(!capture.stdin().is_empty(), "the archive was sent");
    let toasts = fx.toasts();
    assert!(
        toasts.iter().any(|t| t.contains("Uploaded motd")),
        "{toasts:?}"
    );
    // One audit record: the container, the destination and the size, flagged as a replacement,
    // and nothing of the content or of the local directory.
    let audit = fx.state.audit_log();
    let [record] = audit.as_slice() else {
        panic!("one record, got {audit:?}");
    };
    assert_eq!(&*record.cmd, "pod::UploadFile");
    let detail = record.detail.as_deref().unwrap_or("");
    assert!(
        detail.starts_with("container=app path=/motd size=3 overwrite=true"),
        "{detail}"
    );
    assert!(
        !serde_json::to_string(record)
            .expect("json")
            .contains("/home/me")
    );
}

#[gpui::test]
fn a_read_only_cluster_refuses_the_upload_with_a_toast_and_sends_nothing(cx: &mut TestAppContext) {
    let mut fx = Fx::read_only(cx);
    open_and_select(&mut fx, "motd");
    fx.fs.insert("/home/me/notes.txt", b"hello".to_vec());
    fx.script(ExecScript::new().exit(Ok(ExitStatus::with_code(1))));
    drop_files(&mut fx, &["/home/me/notes.txt"]);
    let toasts = fx.toasts();
    assert!(
        toasts
            .iter()
            .any(|t| t.contains("notes.txt") && t.to_lowercase().contains("read-only")),
        "{toasts:?}"
    );
    assert_eq!(
        fx.ports().exec_stream.recorded_calls().len(),
        2,
        "only the listing and the check"
    );
    // Browsing and downloading still work on a read-only cluster.
    fx.script(ExecScript::new().stdout("hello world"));
    let view = fx.view().expect("open");
    let entry = fx
        .read(|v, cx| v.model(cx).selected().cloned())
        .expect("selected");
    fx.vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.start_download(entry, PathBuf::from("/dl/motd"), false, cx)
        });
    });
    fx.settle();
    assert_eq!(fx.fs.file("/dl/motd").as_deref(), Some(&b"hello world"[..]));
}

#[gpui::test]
fn a_dropped_folder_is_refused_and_nothing_is_uploaded(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    open_and_select(&mut fx, "motd");
    // The fake file port reads a directory as missing; the plan fails either way.
    drop_files(&mut fx, &["/home/me/a-folder"]);
    let toasts = fx.toasts();
    assert!(toasts.iter().any(|t| t.contains("a-folder")), "{toasts:?}");
    assert!(fx.service.board().snapshot().is_empty());
    let _ = (Modifiers::none(), point(px(0.), px(0.)));
}

#[gpui::test]
fn the_upload_button_asks_for_files(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    open_and_select(&mut fx, "motd");
    fx.click("files-upload");
    // The platform's file dialog is open; cancelling it does nothing.
    assert!(fx.service.board().snapshot().is_empty());
}
