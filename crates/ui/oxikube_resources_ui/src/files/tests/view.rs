//! The browser as a tab: opening, listing, navigation, sort, hidden files, containers, the empty
//! states, and a 10 000-entry directory.

use gpui::TestAppContext;
use oxikube_app::files::FileKind;
use oxikube_ports::ExitStatus;
use oxikube_testkit::{ExecScript, ExecStreamCall};

use super::{Fx, ls_output, web};
use crate::files::LoadState;
use crate::files::model::{FileColumn, FileSort};

fn listing() -> String {
    ls_output(&[
        ('d', "etc", 4096),
        ('d', "var", 4096),
        ('-', "hello world.txt", 12),
        ('-', ".profile", 33),
        ('-', "motd", 11),
    ])
}

#[gpui::test]
fn browse_files_opens_a_tab_with_the_directory_listed(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    fx.prepare(&["app"], &listing());
    let view = fx.open_at(None);
    assert_eq!(fx.read(|v, _| v.state().clone()), LoadState::Ready);
    assert_eq!(fx.read(|v, _| v.path().to_owned()), "/");
    assert_eq!(
        fx.read(|v, _| v.container().map(str::to_owned)).as_deref(),
        Some("app")
    );
    // Directories first, dot files hidden.
    assert_eq!(fx.names(), ["etc", "var", "hello world.txt", "motd"]);
    let title = fx
        .vcx
        .update(|_, cx| oxikube_workspace::Item::tab_content(view.read(cx), cx).title);
    assert_eq!(title.as_ref(), "Files: web-0");
    assert!(fx.drawn("files-view") && fx.drawn("files-table") && fx.drawn("files-toolbar"));
    // The exec ran `ls` without a TTY in the container.
    let calls = fx.ports().exec_stream.recorded_calls();
    let ExecStreamCall::Exec {
        namespace,
        pod,
        options,
        ..
    } = &calls[0]
    else {
        panic!("an exec");
    };
    assert_eq!((namespace.as_str(), pod.as_str()), ("shop", "web-0"));
    assert!(!options.tty);
    assert_eq!(options.container.as_deref(), Some("app"));
    // Opening it again shows the open one.
    fx.run(oxikube_domain::command::Command::PodBrowseFiles {
        target: web(),
        container: None,
        path: None,
    });
    let open = fx.workspace.read_with(&fx.vcx, |ws, _| {
        ws.items_of_type::<crate::files::FilesView>().len()
    });
    assert_eq!(open, 1, "one browser per pod");
    assert_eq!(
        fx.dispatcher.sent().len(),
        2,
        "both opens went through the dispatcher"
    );
}

#[gpui::test]
fn navigation_uses_the_cache_and_refresh_lists_again(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    fx.prepare(&["app"], &listing());
    let view = fx.open_at(None);
    let execs = |fx: &Fx| fx.ports().exec_stream.recorded_calls().len();
    assert_eq!(execs(&fx), 1);

    // Into /etc: a second exec, with the directory as the script's argument.
    fx.script(ExecScript::new().stdout(ls_output(&[('-', "passwd", 100), ('d', "ssl", 4096)])));
    let row = fx
        .read(|v, cx| v.model(cx).rows().position(|r| r.entry.name == "etc"))
        .unwrap();
    fx.vcx
        .update(|_, cx| view.update(cx, |v, cx| v.open_row(row, cx)));
    fx.settle();
    assert_eq!(fx.read(|v, _| v.path().to_owned()), "/etc");
    assert_eq!(fx.names(), ["ssl", "passwd"]);
    let calls = fx.ports().exec_stream.recorded_calls();
    let ExecStreamCall::Exec { command, .. } = &calls[1] else {
        panic!("an exec")
    };
    assert_eq!(command.last().map(String::as_str), Some("/etc/"));

    // Up and back down come from the cache: no more execs.
    fx.vcx.update(|_, cx| view.update(cx, |v, cx| v.go_up(cx)));
    fx.settle();
    assert_eq!(fx.read(|v, _| v.path().to_owned()), "/");
    assert_eq!(execs(&fx), 2, "the root is cached");
    // The breadcrumb's parts and the up button are drawn.
    assert!(fx.drawn("files-path") && fx.drawn("files-up"));

    // Refresh lists again.
    fx.script(ExecScript::new().stdout(ls_output(&[('-', "new.txt", 1)])));
    fx.click("files-refresh");
    assert_eq!(execs(&fx), 3);
    assert_eq!(fx.names(), ["new.txt"]);
}

#[gpui::test]
fn a_typed_path_navigates_and_the_last_path_is_remembered_per_pod(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    fx.prepare(&["app"], &listing());
    let view = fx.open_at(Some("/var/log/"));
    assert_eq!(
        fx.read(|v, _| v.path().to_owned()),
        "/var/log",
        "a trailing slash is dropped"
    );

    fx.script(ExecScript::new().stdout(ls_output(&[('-', "a", 1)])));
    fx.vcx.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.start_editing_path(window, cx);
            v.path_input
                .update(cx, |input, cx| input.set_value("/tmp", window, cx));
            v.commit_path(window, cx);
        });
    });
    fx.settle();
    assert_eq!(fx.read(|v, _| v.path().to_owned()), "/tmp");
    assert_eq!(fx.service.last_path(&web(), "app").as_deref(), Some("/tmp"));
}

#[gpui::test]
fn sort_by_column_and_the_hidden_toggle(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    fx.prepare(&["app"], &listing());
    let view = fx.open_at(None);
    assert_eq!(fx.names(), ["etc", "var", "hello world.txt", "motd"]);

    // Size, largest first (directories stay on top).
    fx.vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.table.update(cx, |d| {
                d.model.set_sort(FileSort {
                    column: FileColumn::Size,
                    descending: true,
                });
            });
        });
    });
    assert_eq!(fx.names(), ["var", "etc", "hello world.txt", "motd"]);

    // Hidden files appear with the toggle, and the button says so.
    fx.click("files-hidden");
    assert!(fx.names().contains(&".profile".to_owned()));
    assert!(fx.read(|v, cx| v.model(cx).show_hidden()));
    fx.click("files-hidden");
    assert!(!fx.names().contains(&".profile".to_owned()));
}

#[gpui::test]
fn a_pod_with_several_containers_gets_a_selector_and_each_lists_its_own_files(
    cx: &mut TestAppContext,
) {
    let mut fx = Fx::new(cx);
    fx.prepare(&["app", "sidecar"], &listing());
    fx.open_at(None);
    assert!(fx.drawn("files-container-app") && fx.drawn("files-container-sidecar"));

    fx.script(ExecScript::new().stdout(ls_output(&[('-', "envoy.yaml", 10)])));
    fx.click("files-container-sidecar");
    assert_eq!(
        fx.read(|v, _| v.container().map(str::to_owned)).as_deref(),
        Some("sidecar")
    );
    assert_eq!(fx.names(), ["envoy.yaml"]);
    let calls = fx.ports().exec_stream.recorded_calls();
    let ExecStreamCall::Exec { options, .. } = calls.last().unwrap() else {
        panic!("an exec")
    };
    assert_eq!(options.container.as_deref(), Some("sidecar"));

    // A pod with one container shows no selector.
    let mut single = Fx::new(cx);
    single.prepare(&["app"], &listing());
    single.open_at(None);
    assert!(!single.drawn("files-container-app"));
}

#[gpui::test]
fn a_container_without_a_shell_shows_the_clear_state_and_a_missing_directory_an_error(
    cx: &mut TestAppContext,
) {
    let mut fx = Fx::new(cx);
    fx.ports()
        .resources
        .script()
        .get
        .push_ok(super::pod_with(&["app"]));
    fx.script(ExecScript::new().exit(Ok(ExitStatus::with_code(127))));
    fx.script(ExecScript::new().exit(Ok(ExitStatus::with_code(127))));
    fx.open_at(None);
    let state = fx.read(|v, _| v.state().clone());
    let LoadState::NoShell(text) = state else {
        panic!("no shell: {state:?}")
    };
    assert!(text.contains("debug container"), "{text}");
    assert!(fx.drawn("files-no-shell"));
    assert!(!fx.drawn("files-table"));

    let mut fx = Fx::new(cx);
    fx.prepare(&["app"], &listing());
    let view = fx.open_at(None);
    fx.script(
        ExecScript::new()
            .stderr("ls: cannot open directory '/root/': Permission denied")
            .exit(Ok(ExitStatus::with_code(2))),
    );
    fx.vcx
        .update(|_, cx| view.update(cx, |v, cx| v.navigate("/root", cx)));
    fx.settle();
    let LoadState::Failed(text) = fx.read(|v, _| v.state().clone()) else {
        panic!("failed")
    };
    assert!(text.contains("/root"), "{text}");
    assert!(
        fx.drawn("files-failed") && fx.drawn("files-path"),
        "the path bar stays to go back"
    );
}

#[gpui::test]
fn browsing_a_secret_mount_shows_a_warning_line(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    fx.prepare(&["app"], &ls_output(&[('-', "token", 900)]));
    fx.open_at(Some("/var/run/secrets/kubernetes.io/serviceaccount"));
    assert!(fx.drawn("files-secret-warning"));
    assert_eq!(fx.names(), ["token"]);

    let mut ordinary = Fx::new(cx);
    ordinary.prepare(&["app"], &listing());
    ordinary.open_at(None);
    assert!(!ordinary.drawn("files-secret-warning"));
}

#[gpui::test]
fn a_ten_thousand_entry_directory_is_listed_and_only_the_visible_rows_are_drawn(
    cx: &mut TestAppContext,
) {
    let mut fx = Fx::new(cx);
    let mut big = String::from("total 80000\n");
    for i in 0..10_000 {
        big.push_str(&format!(
            "-rw-r--r-- 1 root root {i} 2024-03-05 12:34:56.000000000 +0000 file-{i:05}.log\n"
        ));
    }
    fx.prepare(&["app"], &big);
    fx.open_at(Some("/var/log"));
    assert_eq!(fx.read(|v, cx| v.model(cx).len()), 10_000);
    assert!(fx.drawn("files-table"));
    let visible = fx.read(|v, cx| v.table.visible_rows(cx));
    assert!(
        visible.len() < 100,
        "the table draws the rows in view, not 10 000: {visible:?}"
    );
    assert_eq!(
        fx.read(|v, cx| v.model(cx).row(9_999).map(|r| r.entry.kind)),
        Some(FileKind::File)
    );
}

#[gpui::test]
fn find_fallback_listings_say_names_only(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    fx.ports()
        .resources
        .script()
        .get
        .push_ok(super::pod_with(&["app"]));
    fx.script(ExecScript::new().exit(Ok(ExitStatus::with_code(127))));
    fx.script(ExecScript::new().stdout("d /srv/data\nf /srv/index.html\n"));
    fx.open_at(Some("/srv"));
    assert_eq!(fx.names(), ["data", "index.html"]);
    assert_eq!(fx.read(|v, _| v.state().clone()), LoadState::Ready);
}

#[gpui::test]
fn an_upload_to_another_directory_drops_its_cached_listing(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    fx.prepare(&["app"], &listing());
    let view = fx.open_at(None);
    let execs = |fx: &Fx| fx.ports().exec_stream.recorded_calls().len();
    // Into /etc, so the root's listing is cached and the view is elsewhere.
    fx.script(ExecScript::new().stdout(ls_output(&[('-', "passwd", 100)])));
    let row = fx
        .read(|v, cx| v.model(cx).rows().position(|r| r.entry.name == "etc"))
        .unwrap();
    fx.vcx
        .update(|_, cx| view.update(cx, |v, cx| v.open_row(row, cx)));
    fx.settle();
    let container: std::sync::Arc<str> = "app".into();
    let before = execs(&fx);
    // An upload to / finished while /etc is on show: no reload of /etc ...
    fx.vcx.update(|_, cx| {
        view.update(cx, |v, cx| v.uploaded_to(&container, "/", cx));
    });
    fx.settle();
    assert_eq!(execs(&fx), before);
    assert_eq!(fx.read(|v, _| v.path().to_owned()), "/etc");
    // ... but going back to / lists it again rather than serving the stale cache.
    fx.script(ExecScript::new().stdout(ls_output(&[('-', "uploaded.txt", 1)])));
    fx.vcx.update(|_, cx| view.update(cx, |v, cx| v.go_up(cx)));
    fx.settle();
    assert_eq!(execs(&fx), before + 1);
    assert_eq!(fx.names(), ["uploaded.txt"]);
}

#[test]
fn the_ticker_runs_while_a_batch_is_in_flight_even_with_an_empty_board() {
    use crate::files::view::progress::keep_ticking;
    assert!(keep_ticking(1, &[]));
    assert!(!keep_ticking(0, &[]));
}
