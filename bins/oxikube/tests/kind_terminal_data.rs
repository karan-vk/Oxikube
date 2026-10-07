//! What a terminal leaves on disk (E09-S13, epic E09 "done when" 5): a pod shell and a local shell
//! in the app, typed commands and printed output (a token, a canary), the layout saved with the
//! local terminal's tab, the tabs and the connection closed, and every file of the app's data dir
//! (the state db, its write-ahead log, the settings) scanned for any of it. Terminal content is
//! never persisted: the layout holds a descriptor, and the pod terminal is not saved at all.
//!
//! `cargo test -p oxikube --features integration --test kind_terminal_data` with
//! `OXIKUBE_TEST_CONTEXT` set (`cargo xtask kind-up`); without it the test returns at once.
#![cfg(feature = "integration")]

mod kind_common;

use std::time::Duration;

use gpui::TestAppContext;
use oxikube::app_state::AppState;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_terminal::view::{TERMINAL_ITEM_KIND, TerminalView};
use oxikube_testkit::integration::{TestNamespace, ensure_kind_context, test_context};
use oxikube_workspace::cluster_tab::cluster_layout_key;
use oxikube_workspace::persistence::{LayoutStore, LoadOutcome};

use kind_common::{
    Launched, create_pod, launch_with, open_pod_shell, screen, type_into, wait, wait_value,
};

/// Every file under `dir` with its bytes, recursively: the state db, its write-ahead log and
/// shared-memory file, the settings, anything else the app wrote.
fn files_under(dir: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next).expect("a readable dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let bytes = std::fs::read(&path).unwrap_or_default();
                found.push((path, bytes));
            }
        }
    }
    found
}

/// Whether `bytes` contain `needle`.
fn contains(bytes: &[u8], needle: &str) -> bool {
    let needle = needle.as_bytes();
    bytes.windows(needle.len()).any(|window| window == needle)
}

#[gpui::test]
fn nothing_typed_or_printed_in_a_terminal_reaches_the_data_dir(cx: &mut TestAppContext) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    cx.executor().allow_parking();
    let ns = TestNamespace::create(&context).expect("namespace");
    create_pod(&context, ns.name(), "secrets");
    // A POSIX shell for the local terminal, whatever the developer's login shell is.
    let Launched {
        mut vcx,
        tab,
        cluster,
        table,
        _dir,
    } = launch_with(
        cx,
        &context,
        serde_json::json!({ "terminal": { "shell": "/bin/sh" } }),
    );
    let target = ResourceRef::namespaced(
        cluster.clone(),
        Gvk::new("", "v1", "Pod"),
        ns.name(),
        "secrets",
    );
    let app = vcx.update(|_, cx| AppState::global(cx));

    // A pod shell and a local shell (`terminal::New`, with the cluster's environment).
    let pod_terminal = open_pod_shell(&mut vcx, &tab, &table, &target);
    wait(&mut vcx, "the pod shell to open", |vcx| {
        screen(vcx, &pod_terminal).contains("using sh")
    });
    let bus = app.command_bus().expect("the command bus").clone();
    let outcome = futures::executor::block_on(bus.dispatch(
        Command::TerminalNew {
            cluster: Some(cluster.clone()),
        },
        DispatchContext::new(Initiator::Ui, "kind-test"),
    ));
    assert!(outcome.is_ok(), "{outcome:?}");
    let inner = vcx.update(|_, cx| tab.read(cx).workspace().clone());
    let local_terminal = wait_value(&mut vcx, "the local terminal to open", |vcx| {
        vcx.update(|_, cx| {
            inner
                .read(cx)
                .items_of_type::<TerminalView>()
                .into_iter()
                .find(|view| view.read(cx).descriptor().is_local())
        })
    });

    // What the user types and what the shells print. Each marker appears on screen only once the
    // shell computed it, so the wait proves the bytes went through the grid.
    let typed = [
        "export OXI_API_TOKEN=hunter2-oxi",
        "echo canary-line-$((6*7))",
        "echo bearer-eyJ$((1+1))oxi",
    ];
    for terminal in [&pod_terminal, &local_terminal] {
        wait(&mut vcx, "the terminal's process to start", |vcx| {
            vcx.update(|_, cx| terminal.read(cx).terminal().is_some())
        });
        for line in typed {
            type_into(&mut vcx, terminal, &format!("{line}\n"));
        }
        wait(&mut vcx, "the output on screen", |vcx| {
            let text = screen(vcx, terminal);
            text.contains("canary-line-42") && text.contains("bearer-eyJ2oxi")
        });
    }

    // The cluster tab's layout is saved (500 ms debounce on the test clock): the local terminal's
    // descriptor is in it, the pod terminal (never restored on its own) and everything either
    // showed are not.
    vcx.executor().advance_clock(Duration::from_secs(2));
    vcx.run_until_parked();
    let layout = LayoutStore::new(app.state().clone(), &cluster_layout_key(&cluster))
        .expect("the cluster's layout key");
    let saved = match futures::executor::block_on(layout.load()).expect("the layout loads") {
        LoadOutcome::Loaded(layout) => serde_json::to_string(&layout.to_json()).unwrap(),
        other => panic!("the cluster tab's layout was saved: {other:?}"),
    };
    assert_eq!(
        saved.matches(TERMINAL_ITEM_KIND).count(),
        1,
        "the local terminal is saved, the pod terminal is not: {saved}"
    );

    let forbidden = [
        "hunter2",
        "OXI_API_TOKEN",
        "canary-line",
        "bearer-eyJ",
        "using sh",
        "echo ",
        "secrets", // the pod the shell ran in is the audit's business, not the layout's
    ];
    for needle in &forbidden[..forbidden.len() - 1] {
        assert!(
            !saved.contains(needle),
            "{needle} is in the saved layout: {saved}"
        );
    }
    let files = files_under(_dir.path());
    let state_db = files
        .iter()
        .find(|(path, _)| path.ends_with("state.db"))
        .expect("the state db is among the scanned files");
    // The scan can see text: the guard's audit record of the shell open is in the db.
    assert!(
        files.iter().any(|(_, bytes)| contains(bytes, "pod::Shell")) && !state_db.1.is_empty(),
        "the audit record of the shell is readable in the data dir"
    );
    let leaks = |files: &[(std::path::PathBuf, Vec<u8>)]| {
        files
            .iter()
            .flat_map(|(path, bytes)| {
                forbidden[..forbidden.len() - 1]
                    .iter()
                    .filter(|needle| contains(bytes, needle))
                    .map(move |needle| format!("{needle} in {}", path.display()))
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(leaks(&files), Vec::<String>::new());

    // Closing both tabs and the connection leaves nothing behind either.
    vcx.update(|window, cx| {
        for id in [pod_terminal.entity_id(), local_terminal.entity_id()] {
            inner.update(cx, |ws, cx| ws.close_item(id, window, cx));
        }
    });
    vcx.executor().advance_clock(Duration::from_secs(2));
    vcx.run_until_parked();
    app.services().sessions.disconnect(&cluster).ok();
    vcx.run_until_parked();
    assert_eq!(leaks(&files_under(_dir.path())), Vec::<String>::new());
}
