//! The buttons: each sends its command; removal asks first and says what it will do.

use std::path::PathBuf;

use gpui::TestAppContext;
use oxikube_domain::command::{Command, KubeconfigSourceRef, NewKubeconfigSource};
use oxikube_ports::UserSource;

use super::super::test_support::found;
use super::super::view::removal_text;
use super::Harness;

fn add(source: NewKubeconfigSource) -> Command {
    Command::KubeconfigAddSource { source }
}

#[gpui::test]
fn reload_all_sends_the_reload_command(cx: &mut TestAppContext) {
    let (mut h, backend) = Harness::scripted(cx, vec![found(UserSource::file("/a.yaml"), 1)]);
    h.click("sources-reload");
    assert_eq!(backend.sent(), vec![Command::KubeconfigReload]);
}

#[gpui::test]
fn add_file_opens_the_platform_picker_and_adds_every_chosen_file(cx: &mut TestAppContext) {
    let (mut h, backend) = Harness::scripted(cx, vec![found(UserSource::default_source(), 1)]);
    h.click("sources-add-file");
    assert!(
        cx.did_prompt_for_paths(),
        "the platform picker was asked for"
    );
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && !options.directories, "files only");
        assert!(options.multiple);
        Some(vec![
            PathBuf::from("/work/a.yaml"),
            PathBuf::from("/work/b.yaml"),
        ])
    });
    h.vcx.run_until_parked();
    assert_eq!(
        backend.sent(),
        vec![
            add(NewKubeconfigSource::File {
                path: "/work/a.yaml".into()
            }),
            add(NewKubeconfigSource::File {
                path: "/work/b.yaml".into()
            }),
        ]
    );
}

#[gpui::test]
fn add_folder_picks_directories(cx: &mut TestAppContext) {
    let (mut h, backend) = Harness::scripted(cx, vec![]);
    h.click("sources-add-folder");
    cx.simulate_path_prompt_response(|options| {
        assert!(options.directories && !options.files, "folders only");
        Some(vec![PathBuf::from("/work/configs")])
    });
    h.vcx.run_until_parked();
    assert_eq!(
        backend.sent(),
        vec![add(NewKubeconfigSource::Dir {
            path: "/work/configs".into()
        })]
    );
}

#[gpui::test]
fn cancelling_the_picker_sends_nothing(cx: &mut TestAppContext) {
    let (mut h, backend) = Harness::scripted(cx, vec![]);
    h.click("sources-add-file");
    cx.simulate_path_prompt_response(|_| None);
    h.vcx.run_until_parked();
    assert!(backend.sent().is_empty());
    assert!(!h.read(|view| view.is_busy()));
}

#[gpui::test]
fn removing_a_user_owned_file_asks_and_says_the_file_is_not_touched(cx: &mut TestAppContext) {
    let (mut h, backend) = Harness::scripted(
        cx,
        vec![
            found(UserSource::default_source(), 1),
            found(UserSource::file("/work/prod.yaml"), 2),
        ],
    );
    h.click("sources-remove-1");
    assert!(h.modal_open(), "the confirmation is open");
    assert!(h.is_laid_out("dialog-modal"));
    assert!(
        backend.sent().is_empty(),
        "nothing is sent before the answer"
    );

    // Cancelling changes nothing.
    h.click("dialog-cancel");
    assert!(!h.modal_open());
    assert!(backend.sent().is_empty());

    h.click("sources-remove-1");
    h.click("dialog-confirm");
    assert_eq!(
        backend.sent(),
        vec![Command::KubeconfigRemoveSource {
            source: KubeconfigSourceRef::File {
                path: "/work/prod.yaml".into()
            }
        }]
    );
}

#[gpui::test]
fn removing_a_stored_kubeconfig_is_confirmed_as_a_deletion(cx: &mut TestAppContext) {
    let stored = PathBuf::from("/config/kubeconfigs/prod.yaml");
    let (mut h, backend) = Harness::scripted(cx, vec![found(UserSource::file(&stored), 1)]);
    h.click("sources-remove-0");
    h.click("dialog-confirm");
    assert_eq!(
        backend.sent(),
        vec![Command::KubeconfigRemoveSource {
            source: KubeconfigSourceRef::File {
                path: stored.display().to_string()
            }
        }]
    );
}

#[test]
fn the_confirmation_text_tells_a_deletion_from_a_list_edit() {
    let owned = removal_text(&UserSource::file("/work/prod.yaml"), false);
    assert!(owned.contains("not touched"), "{owned}");
    assert!(owned.contains("yours"), "{owned}");
    let folder = removal_text(&UserSource::dir("/work"), false);
    assert!(folder.contains("not touched"), "{folder}");
    let default = removal_text(&UserSource::default_source(), false);
    assert!(default.contains("add this entry back"), "{default}");
    let stored = removal_text(&UserSource::file("/config/kubeconfigs/a.yaml"), true);
    assert!(stored.contains("deletes that file"), "{stored}");
    assert!(!stored.contains("not touched"), "{stored}");
}
