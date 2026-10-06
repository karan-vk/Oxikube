//! What the list shows: inline errors, the empty explainer, the loading state, the tab.

use gpui::TestAppContext;
use oxikube_ports::{SourceState, UserSource};
use oxikube_workspace::{Item as _, OpenOptions};

use super::super::model::LoadState;
use super::super::test_support::{ScriptedBackend, broken, found, row};
use super::super::view::{EMPTY_STEPS, EMPTY_TITLE};
use super::super::{SourcesDeps, SourcesView};
use super::Harness;
use gpui::AppContext as _;

#[gpui::test]
fn an_invalid_file_shows_an_inline_error_while_other_sources_stay_listed(cx: &mut TestAppContext) {
    let (mut h, _) = Harness::scripted(
        cx,
        vec![
            found(UserSource::default_source(), 3),
            found(UserSource::file("/work/good.yaml"), 2),
            broken(
                UserSource::file("/work/broken.yaml"),
                SourceState::Invalid,
                "Not a valid kubeconfig",
            ),
            row(
                UserSource::dir("/work/configs"),
                Some(SourceState::Found),
                4,
                Some("1 of 5 files skipped: bad.yaml: not a valid kubeconfig"),
            ),
        ],
    );
    for ix in 0..4 {
        assert!(h.is_laid_out(&format!("sources-row-{ix}")), "row {ix}");
    }
    // The error is tagged on its own row only; the sources next to it are not marked.
    assert!(h.is_laid_out("sources-status-2:error"));
    for ix in [0, 1, 3] {
        assert!(
            !h.is_laid_out(&format!("sources-status-{ix}:error")),
            "row {ix} has no error"
        );
        assert!(h.is_laid_out(&format!("sources-status-{ix}")));
    }
    h.read(|view| {
        let rows = view.model().rows();
        assert_eq!(rows[2].message.as_deref(), Some("Not a valid kubeconfig"));
        assert_eq!(rows[1].contexts, 2);
        assert_eq!(rows[3].contexts, 4);
    });
    assert!(h.is_laid_out("sources-count:4 sources, 1 with a problem"));
}

#[gpui::test]
fn an_empty_list_shows_the_explainer(cx: &mut TestAppContext) {
    let (mut h, backend) = Harness::scripted(cx, vec![]);
    assert!(h.is_laid_out("sources-empty"));
    for ix in 0..EMPTY_STEPS.len() {
        assert!(
            h.is_laid_out(&format!("sources-empty-step-{ix}")),
            "step {ix}"
        );
    }
    assert!(!h.is_laid_out("sources-row-0"));
    assert!(EMPTY_TITLE.contains("No kubeconfig sources"));
    // The header's buttons are there to fix it.
    for button in [
        "sources-add-file",
        "sources-add-folder",
        "sources-paste",
        "sources-reload",
    ] {
        assert!(h.is_laid_out(button), "{button}");
    }
    assert!(backend.sent().is_empty());
}

#[gpui::test]
fn the_empty_explainer_has_a_button_that_adds_the_default_source_back(cx: &mut TestAppContext) {
    use oxikube_domain::command::{Command, NewKubeconfigSource};
    let (mut h, backend) = Harness::scripted(cx, vec![]);
    h.click("sources-add-default");
    assert_eq!(
        backend.sent(),
        vec![Command::KubeconfigAddSource {
            source: NewKubeconfigSource::Default
        }]
    );
}

#[gpui::test]
fn the_first_frame_is_loading_and_the_rows_arrive_without_blocking(cx: &mut TestAppContext) {
    let (workspace, mut vcx) = oxikube_workspace::test_support::open_workspace(cx);
    vcx.update(|_, cx| oxikube_runtime::init_deterministic(cx));
    let backend = ScriptedBackend::new(vec![found(UserSource::file("/a.yaml"), 1)]);
    let deps = SourcesDeps {
        backend: std::rc::Rc::new(backend),
        workspace: Some(workspace.downgrade()),
    };
    let view = vcx.update(|_, cx| cx.new(|cx| SourcesView::new(deps, cx)));
    // Nothing has run yet: no read has finished.
    vcx.update(|_, cx| {
        assert_eq!(view.read(cx).model().load_state(), &LoadState::Loading);
    });
    vcx.run_until_parked();
    vcx.update(|_, cx| {
        assert_eq!(view.read(cx).model().load_state(), &LoadState::Ready);
        assert_eq!(view.read(cx).model().rows().len(), 1);
    });
}

#[gpui::test]
fn the_screen_is_one_tab_and_opening_it_twice_shows_the_open_one(cx: &mut TestAppContext) {
    let (workspace, mut vcx) = oxikube_workspace::test_support::open_workspace(cx);
    vcx.update(|_, cx| oxikube_runtime::init_deterministic(cx));
    let backend = ScriptedBackend::new(vec![found(UserSource::file("/a.yaml"), 1)]);
    for _ in 0..2 {
        let deps = SourcesDeps {
            backend: std::rc::Rc::new(backend.clone()),
            workspace: Some(workspace.downgrade()),
        };
        vcx.update(|window, cx| {
            let view = cx.new(|cx| SourcesView::new(deps, cx));
            workspace.update(cx, |ws, cx| {
                ws.open_item_with(
                    Box::new(view),
                    OpenOptions {
                        reuse_existing: true,
                        ..OpenOptions::default()
                    },
                    window,
                    cx,
                );
            });
        });
        vcx.run_until_parked();
    }
    vcx.update(|_, cx| {
        let views = workspace.read(cx).items_of_type::<SourcesView>();
        assert_eq!(views.len(), 1, "the second open reused the first");
        let tab = views[0].read(cx).tab_content(cx);
        assert_eq!(tab.title.as_ref(), "Kubeconfig sources");
    });
}
