//! The screen over the real service on fakes: the acceptance criteria end to end.

use std::path::PathBuf;

use gpui::TestAppContext;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{
    ClusterContext, ClusterSource, SourceId, SourceKind, SourceState, SourceStatus, SourcesChanged,
    UserSource,
};

use super::{Harness, Services, kubeconfig};

fn status(path: &str, state: SourceState, contexts: usize, message: Option<&str>) -> SourceStatus {
    SourceStatus {
        source: ClusterSource {
            id: SourceId(format!("file:{path}")),
            kind: SourceKind::KubeconfigFile,
            label: path.into(),
            path: Some(PathBuf::from(path)),
        },
        state,
        contexts,
        message: message.map(Into::into),
    }
}

fn rows(h: &mut Harness) -> Vec<(String, Option<SourceState>, usize, bool)> {
    h.read(|view| {
        view.model()
            .rows()
            .iter()
            .map(|r| (r.label.clone(), r.state, r.contexts, r.stored))
            .collect()
    })
}

#[gpui::test]
fn pasting_then_removing_a_kubeconfig_end_to_end(cx: &mut TestAppContext) {
    let services = Services::new([UserSource::default_source()]);
    let mut h = Harness::open(cx, services.backend());
    let stored = "/config/kubeconfigs/prod.yaml";

    // Paste: validated, stored owner-only, listed as a file, told to the cluster source.
    h.click("sources-paste");
    let layer = h
        .workspace
        .read_with(&h.vcx, |ws, _| ws.modal_layer().clone());
    let dialog = h
        .vcx
        .read(|cx| {
            layer
                .read(cx)
                .active_modal::<super::super::paste::PasteDialog>()
        })
        .unwrap();
    h.vcx.update(|window, cx| {
        dialog.update(cx, |d, cx| d.fill("prod", &kubeconfig(2), window, cx));
    });
    services
        .source
        .set_statuses([status(stored, SourceState::Found, 2, None)]);
    h.click("paste-submit");

    assert_eq!(
        services.fs.file(stored),
        Some(kubeconfig(2).into_bytes()),
        "stored as <config>/kubeconfigs/<name>.yaml"
    );
    assert!(
        services.fs.is_private(stored),
        "with owner-only permissions"
    );
    assert_eq!(
        services.list.snapshot(),
        vec![UserSource::default_source(), UserSource::file(stored)]
    );
    assert_eq!(services.source.user_sources(), services.list.snapshot());
    let listed = rows(&mut h);
    assert_eq!(listed.len(), 2);
    assert_eq!(
        listed[1],
        (stored.to_owned(), Some(SourceState::Found), 2, true)
    );
    assert!(h.is_laid_out("sources-row-1"));

    // Remove: confirmed, the file and the entry go.
    h.click("sources-remove-1");
    assert!(h.modal_open());
    h.click("dialog-confirm");
    assert!(
        services.fs.file(stored).is_none(),
        "the stored file was deleted"
    );
    assert_eq!(services.list.snapshot(), vec![UserSource::default_source()]);
    assert_eq!(rows(&mut h).len(), 1);
}

#[gpui::test]
fn removing_a_user_owned_file_only_removes_the_entry(cx: &mut TestAppContext) {
    let services = Services::new([
        UserSource::default_source(),
        UserSource::file("/work/prod.yaml"),
    ]);
    services
        .fs
        .insert("/work/prod.yaml", kubeconfig(1).into_bytes());
    let mut h = Harness::open(cx, services.backend());
    h.click("sources-remove-1");
    h.click("dialog-confirm");
    assert_eq!(services.list.snapshot(), vec![UserSource::default_source()]);
    assert_eq!(
        services.fs.file("/work/prod.yaml"),
        Some(kubeconfig(1).into_bytes()),
        "the user's file is where it was"
    );
    assert_eq!(rows(&mut h).len(), 1);
}

#[gpui::test]
fn an_invalid_file_added_from_the_picker_is_listed_with_its_error(cx: &mut TestAppContext) {
    let services = Services::new([UserSource::default_source()]);
    services.source.set_statuses([
        status("/home/me/.kube/config", SourceState::Found, 3, None),
        status(
            "/work/broken.yaml",
            SourceState::Invalid,
            0,
            Some("Not a valid kubeconfig"),
        ),
    ]);
    let mut h = Harness::open(cx, services.backend());
    h.click("sources-add-file");
    cx.simulate_path_prompt_response(|_| Some(vec![PathBuf::from("/work/broken.yaml")]));
    h.vcx.run_until_parked();

    assert_eq!(
        services.list.snapshot(),
        vec![
            UserSource::default_source(),
            UserSource::file("/work/broken.yaml")
        ]
    );
    // Added anyway: the problem is shown next to it, inline.
    assert!(h.is_laid_out("sources-status-1:error"));
    h.read(|view| {
        assert_eq!(
            view.model().rows()[1].message.as_deref(),
            Some("Not a valid kubeconfig")
        );
    });
}

#[gpui::test]
fn reload_all_rereads_and_shows_what_the_sources_now_say(cx: &mut TestAppContext) {
    let services = Services::new([UserSource::file("/work/a.yaml")]);
    services.source.set_statuses([status(
        "/work/a.yaml",
        SourceState::Invalid,
        0,
        Some("Not a valid kubeconfig"),
    )]);
    let mut h = Harness::open(cx, services.backend());
    assert!(h.is_laid_out("sources-status-0:error"));

    // The user fixes the file, then reloads: the sources are read again and the row follows.
    services
        .source
        .set_statuses([status("/work/a.yaml", SourceState::Found, 2, None)]);
    services
        .source
        .script()
        .reload
        .push_ok(SourcesChanged::default());
    h.click("sources-reload");
    assert!(
        services
            .source
            .recorded_calls()
            .contains(&oxikube_testkit::ClusterSourceCall::Reload),
        "the cluster source was asked to re-read"
    );
    assert!(!h.is_laid_out("sources-status-0:error"));
    assert_eq!(rows(&mut h)[0].1, Some(SourceState::Found));
    assert_eq!(rows(&mut h)[0].2, 2);
}

#[gpui::test]
fn a_catalog_change_refreshes_the_rows_by_itself(cx: &mut TestAppContext) {
    // A file watcher saw a change: the diff arrives on the subscription and the rows re-read.
    let services = Services::new([UserSource::file("/work/a.yaml")]);
    services
        .source
        .set_statuses([status("/work/a.yaml", SourceState::Found, 1, None)]);
    let mut h = Harness::open(cx, services.backend());
    assert_eq!(rows(&mut h)[0].2, 1);

    services
        .source
        .set_statuses([status("/work/a.yaml", SourceState::Found, 5, None)]);
    // The diff a watcher reload would send, delivered to every subscriber.
    let name = ContextName::new("fresh");
    let fresh = ClusterContext::new(
        ClusterId::new("/work/a.yaml", &name),
        name,
        SourceId("file:/work/a.yaml".into()),
    );
    services
        .source
        .script()
        .set_user_sources
        .push_ok(SourcesChanged {
            added: vec![fresh],
            ..SourcesChanged::default()
        });
    futures::executor::block_on(services.service.apply_stored()).unwrap();
    h.vcx.run_until_parked();
    assert_eq!(rows(&mut h)[0].2, 5);
}
