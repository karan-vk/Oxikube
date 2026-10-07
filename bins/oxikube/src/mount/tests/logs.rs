//! The log service of the main window (E08-S01): built by the mount, shared by every window, and
//! bounded by `logs.buffer_lines` through the real settings store. The log viewer (E08-S02): "View
//! Logs" on a pod's row opens its log as a tab of the cluster tab, streamed through the cluster's
//! `LogPort`.

use std::sync::Arc;

use gpui::{TestAppContext, UpdateGlobal as _};
use oxikube_app::logs::{DEFAULT_BUFFER_LINES, LogTarget};
use oxikube_domain::log::LogLine;
use oxikube_logs_ui::LogView;
use oxikube_ports::LogOptions;
use oxikube_resources_ui::table::ResourceTable;
use oxikube_settings::SettingsStore;
use oxikube_testkit::{FakeLogPort, TestPorts, Timeline};

use oxikube_domain::command::CommandId;

use super::App;
use crate::app_state::AppState;

#[gpui::test]
fn the_mount_builds_the_log_service_with_the_default_bound(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let service = app
        .vcx
        .update(|_, cx| AppState::global(cx).log_service().cloned())
        .expect("the mount built the app's log service");
    assert_eq!(service.buffer_lines(), DEFAULT_BUFFER_LINES);
}

#[gpui::test]
fn a_user_setting_bounds_the_service_and_hot_reloads_into_open_sessions(cx: &mut TestAppContext) {
    let mut app = App::start_with(cx, TestPorts::seeded(), |cx| {
        SettingsStore::update_global(cx, |store, _| {
            store
                .set_user_settings(r#"{ "logs": { "buffer_lines": 1500 } }"#)
                .expect("valid settings");
        });
    });
    let service = app
        .vcx
        .update(|_, cx| AppState::global(cx).log_service().cloned())
        .expect("the log service");
    assert_eq!(service.buffer_lines(), 1_500, "built from the user's value");

    // An open session follows an edit made while it streams.
    let port = Arc::new(FakeLogPort::new());
    port.script()
        .stream_logs
        .push_ok(Timeline::new().keep_open());
    let session = service.open(
        port,
        LogTarget::pod("default", "web-0"),
        LogOptions::follow(),
    );
    app.vcx.update(|_, cx| {
        SettingsStore::update_global(cx, |store, _| {
            store
                .set_user_settings(r#"{ "logs": { "buffer_lines": 400 } }"#)
                .expect("valid settings");
        });
    });
    assert_eq!(service.buffer_lines(), 400);
    session.read(|buffer, _| assert_eq!(buffer.capacity(), 400));
}

#[gpui::test]
fn view_logs_on_a_pod_row_opens_its_log_in_the_cluster_tab(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let ports = app.ports.connector.ports_for(&TestPorts::cluster_id());
    let line = |i: i64| {
        LogLine::new(
            jiff::Timestamp::from_second(1_791_115_200 + i).unwrap(),
            "web-running",
            "app",
            format!("hello {i}"),
        )
    };
    ports
        .logs
        .script()
        .stream_logs
        .push_ok(Timeline::immediate((0..3).map(line)));
    app.open_pods_table();

    // The row's actions (its context menu, the palette's list) offer "View Logs" for a pod.
    let ws = app.tab_workspace();
    let table = app
        .vcx
        .update(|_, cx| ws.read(cx).items_of_type::<ResourceTable>().remove(0));
    let (entry, targets) = app.vcx.update(|_, cx| {
        let table = table.read(cx);
        let entry = table
            .action_entries(cx)
            .into_iter()
            .find(|entry| entry.command() == CommandId::POD_VIEW_LOGS);
        (entry, table.action_targets(cx))
    });
    let entry = entry.expect("pods offer View Logs");
    assert_eq!(entry.label, "View Logs");
    assert!(entry.is_enabled());

    app.vcx.update(|window, cx| {
        table.update(cx, |table, cx| {
            table.run_action(CommandId::POD_VIEW_LOGS, targets, window, cx);
        });
    });
    app.tick();
    app.tick();

    let views = app
        .vcx
        .update(|_, cx| ws.read(cx).items_of_type::<LogView>());
    assert_eq!(views.len(), 1, "a log tab opened in the cluster tab");
    let (lines, first, title) = app.vcx.update(|_, cx| {
        let view = views[0].read(cx);
        (
            view.line_window().line_count(),
            view.row_text(0),
            oxikube_workspace::Item::tab_content(view, cx).title,
        )
    });
    assert_eq!(lines, 3, "streamed through the cluster's LogPort");
    assert_eq!(first.as_deref(), Some("hello 0"));
    assert_eq!(title.as_ref(), "web-running/app");
    let streams = ports.logs.recorded_calls();
    assert_eq!(streams.len(), 1);
}

/// The save, copy, mark and clear commands (E08-S06) reach the open log view through the real
/// bus: the view's keys and toolbar send them, the registered handlers queue them for the window's
/// `LogViews`, which applies them.
#[gpui::test]
fn the_local_log_actions_reach_the_view_through_the_bus(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let ports = app.ports.connector.ports_for(&TestPorts::cluster_id());
    let line = |i: i64| {
        LogLine::new(
            jiff::Timestamp::from_second(1_791_115_200 + i).unwrap(),
            "web-running",
            "app",
            format!("hello {i}"),
        )
    };
    ports
        .logs
        .script()
        .stream_logs
        .push_ok(Timeline::immediate((0..3).map(line)));
    app.open_pods_table();
    let ws = app.tab_workspace();
    let table = app
        .vcx
        .update(|_, cx| ws.read(cx).items_of_type::<ResourceTable>().remove(0));
    app.vcx.update(|window, cx| {
        table.update(cx, |table, cx| {
            let targets = table.action_targets(cx);
            table.run_action(CommandId::POD_VIEW_LOGS, targets, window, cx);
        });
    });
    app.tick();
    app.tick();
    let view = app
        .vcx
        .update(|_, cx| ws.read(cx).items_of_type::<LogView>().remove(0));
    let marked = |app: &mut App| app.vcx.update(|_, cx| view.read(cx).marked());

    // Mark: the line on top (nothing was clicked).
    app.vcx
        .update(|_, cx| view.update(cx, |view, cx| view.request_mark(cx)));
    app.tick();
    assert_eq!(marked(&mut app), [0], "logs::Mark reached the view");

    // Copy: what is on screen goes to the clipboard.
    app.vcx
        .update(|_, cx| view.update(cx, |view, cx| view.request_copy(cx)));
    app.tick();
    let copied = app.vcx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("hello 0\nhello 1\nhello 2\n"));

    // Save: the dialog says what would be written; nothing is written before a file is chosen.
    app.vcx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.request_save(oxikube_domain::log::LogSaveScope::All, cx)
        })
    });
    app.tick();
    let summary = app.vcx.update(|_, cx| {
        ws.read(cx)
            .modal_layer()
            .read(cx)
            .active_modal::<oxikube_logs_ui::SaveDialog>()
            .map(|dialog| dialog.read(cx).summary())
    });
    assert_eq!(
        summary.as_deref(),
        Some("Everything the buffer holds: 3 lines.")
    );
    assert!(app.ports.fs.recorded_calls().is_empty());

    // Clear: a marked line makes it ask first (the save dialog gives way to the confirmation).
    app.vcx
        .update(|_, cx| view.update(cx, |view, cx| view.request_clear(cx)));
    app.tick();
    let asked = app.vcx.update(|_, cx| {
        ws.read(cx)
            .modal_layer()
            .read(cx)
            .active_modal::<oxikube_workspace::DialogModal>()
            .is_some()
    });
    assert!(asked, "clearing with a mark asks first");
}
