//! The paste dialog.

use gpui::TestAppContext;
use oxikube_domain::command::{Command, NewKubeconfigSource, PastedText};
use oxikube_ports::UserSource;

use super::super::paste::{PasteDialog, storage_warning};
use super::super::test_support::{STORED_DIR, found};
use super::{Harness, Services, kubeconfig};

const SECRET: &str = "s3cr3t-token-do-not-leak";

/// The open paste dialog.
fn dialog(h: &mut Harness) -> gpui::Entity<PasteDialog> {
    let layer = h
        .workspace
        .read_with(&h.vcx, |ws, _| ws.modal_layer().clone());
    h.vcx
        .read(|cx| layer.read(cx).active_modal::<PasteDialog>())
        .expect("the paste dialog is open")
}

#[test]
fn the_warning_says_where_the_file_goes_and_that_it_may_hold_credentials() {
    let text = storage_warning(std::path::Path::new(STORED_DIR));
    assert!(text.contains(STORED_DIR), "{text}");
    assert!(text.contains("readable by you only"), "{text}");
    assert!(text.contains("credentials"), "{text}");
}

#[gpui::test]
fn paste_opens_a_dialog_with_name_text_and_the_credentials_warning(cx: &mut TestAppContext) {
    let (mut h, _) = Harness::scripted(cx, vec![found(UserSource::default_source(), 1)]);
    h.click("sources-paste");
    assert!(h.modal_open());
    for part in [
        "paste-dialog",
        "paste-name",
        "paste-text",
        "paste-warning",
        "paste-submit",
    ] {
        assert!(h.is_laid_out(part), "{part}");
    }
    assert!(!h.is_laid_out("paste-error"));
}

#[gpui::test]
fn submitting_sends_the_pasted_text_as_one_command_and_closes(cx: &mut TestAppContext) {
    let (mut h, backend) = Harness::scripted(cx, vec![]);
    h.click("sources-paste");
    let dialog = dialog(&mut h);
    h.vcx.update(|window, cx| {
        dialog.update(cx, |dialog, cx| {
            dialog.fill("prod", &kubeconfig(2), window, cx)
        });
    });
    h.click("paste-submit");
    assert_eq!(
        backend.sent(),
        vec![Command::KubeconfigAddSource {
            source: NewKubeconfigSource::Pasted {
                name: "prod".into(),
                text: PastedText::new(kubeconfig(2)),
            }
        }]
    );
    assert!(!h.modal_open(), "the dialog closed after a success");
    h.read(|view| {
        assert_eq!(view.model().notice().map(|n| n.error), Some(false));
    });
}

#[gpui::test]
fn an_empty_name_or_text_is_explained_and_nothing_is_sent(cx: &mut TestAppContext) {
    let (mut h, backend) = Harness::scripted(cx, vec![]);
    h.click("sources-paste");
    let dialog = dialog(&mut h);
    h.click("paste-submit");
    assert!(h.is_laid_out("paste-error"), "asks for a name");
    h.vcx.update(|window, cx| {
        dialog.update(cx, |dialog, cx| dialog.fill("prod", "", window, cx));
    });
    h.click("paste-submit");
    h.vcx.read(|cx| {
        let error = dialog.read(cx).error().expect("an error").to_string();
        assert!(error.contains("Paste"), "{error}");
    });
    assert!(backend.sent().is_empty());
    assert!(h.modal_open());
}

#[gpui::test]
fn invalid_text_is_an_inline_error_and_the_dialog_keeps_the_text(cx: &mut TestAppContext) {
    // The real service over fakes: validation runs through the cluster source port.
    let services = Services::new([UserSource::default_source()]);
    let mut h = Harness::open(cx, services.backend());
    h.click("sources-paste");
    let dialog = dialog(&mut h);
    let bad = format!("this is: [not a kubeconfig\ntoken: {SECRET}");
    h.vcx.update(|window, cx| {
        dialog.update(cx, |dialog, cx| dialog.fill("prod", &bad, window, cx));
    });
    h.click("paste-submit");

    assert!(h.modal_open(), "the dialog stays open on an error");
    assert!(h.is_laid_out("paste-error"));
    let shown = h
        .vcx
        .read(|cx| dialog.read(cx).error().map(|e| e.to_string()));
    let shown = shown.expect("an error is shown");
    assert!(shown.contains("not a valid kubeconfig"), "{shown}");
    assert!(!shown.contains(SECRET), "the error never quotes the text");
    assert!(!h.vcx.read(|cx| dialog.read(cx).is_busy()));
    // Nothing was stored or listed.
    assert!(services.fs.file("/config/kubeconfigs/prod.yaml").is_none());
    assert_eq!(services.list.snapshot(), vec![UserSource::default_source()]);

    // Fixing the text and submitting again works, in the same dialog.
    h.vcx.update(|window, cx| {
        dialog.update(cx, |dialog, cx| {
            dialog.fill("prod", &kubeconfig(1), window, cx)
        });
    });
    h.click("paste-submit");
    assert!(!h.modal_open());
    assert!(services.fs.file("/config/kubeconfigs/prod.yaml").is_some());
}

#[gpui::test]
fn an_existing_name_is_a_conflict_shown_in_the_dialog(cx: &mut TestAppContext) {
    let services = Services::new([]);
    services
        .fs
        .insert("/config/kubeconfigs/prod.yaml", b"original".to_vec());
    let mut h = Harness::open(cx, services.backend());
    h.click("sources-paste");
    let dialog = dialog(&mut h);
    h.vcx.update(|window, cx| {
        dialog.update(cx, |dialog, cx| {
            dialog.fill("prod", &kubeconfig(1), window, cx)
        });
    });
    h.click("paste-submit");
    assert!(h.modal_open());
    let shown = h
        .vcx
        .read(|cx| dialog.read(cx).error().map(|e| e.to_string()))
        .unwrap();
    assert!(shown.contains("already exists"), "{shown}");
    assert_eq!(
        services.fs.file("/config/kubeconfigs/prod.yaml"),
        Some(b"original".to_vec())
    );
}
