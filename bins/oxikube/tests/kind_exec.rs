//! A shell in a pod, end to end (E09-S08): the real init order with the app's own adapters, its
//! main window headless, a busybox pod in a namespace of the test's own, and the user's path:
//! connect the kind context, open the pods table, choose "Shell" on the pod. The pod is read
//! through the connection, the command goes through the bus and the guard (audited in the SQLite
//! state db), a terminal tab opens in the cluster tab's bottom dock, the exec probe finds that
//! busybox has no `bash`, an `sh` session opens over the real websocket (the terminal's first
//! line says so) and a typed command's output lands in the grid.
//!
//! The second test is the same path for a debug container (E09-S10): "Debug" on the pod's row opens the
//! dialog, its button runs `pod::Debug` through the bus and the guard (confirmed on the dialog's
//! behalf, audited with the image and target), the ephemeral container is added to the pod, a terminal
//! tab attached to it opens in the bottom dock, and `ps` in it shows the target's process.
//!
//! `cargo test -p oxikube --features integration --test kind_exec` with `OXIKUBE_TEST_CONTEXT`
//! set (`cargo xtask kind-up`); without it the test returns at once. The pod and its namespace
//! are deleted at the end. Real I/O wakes GPUI tasks from Tokio threads, so the test allows
//! parking and polls with short real-time sleeps.
#![cfg(feature = "integration")]

mod kind_common;

use gpui::TestAppContext;
use oxikube::app_state::AppState;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_ports::AuditQuery;
use oxikube_resources_ui::exec::DebugDialog;
use oxikube_terminal::view::{BackendDescriptor, TerminalPanel, TerminalView};
use oxikube_testkit::images::BUSYBOX;
use oxikube_testkit::integration::{TestNamespace, ensure_kind_context, test_context};
use oxikube_workspace::DockPosition;

use kind_common::{Launched, create_pod, ephemeral_containers, launch, screen, wait, wait_value};

#[gpui::test]
fn a_shell_opens_in_a_pod_from_its_row_through_the_guard_and_runs_a_command(
    cx: &mut TestAppContext,
) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    cx.executor().allow_parking();

    let ns = TestNamespace::create(&context).expect("namespace");
    create_pod(&context, ns.name(), "shell-target");
    let Launched {
        mut vcx,
        tab,
        cluster,
        table,
        _dir,
    } = launch(cx, &context);

    // "Shell" on the pod's row: the menu item and the `s` key both end in this.
    let target = ResourceRef::namespaced(
        cluster.clone(),
        Gvk::new("", "v1", "Pod"),
        ns.name(),
        "shell-target",
    );
    vcx.update(|window, cx| {
        table.update(cx, |table, cx| {
            let entries = table.action_entries(cx);
            assert!(
                entries
                    .iter()
                    .any(|e| e.command() == CommandId::POD_SHELL && e.is_enabled()),
                "the pods table offers Shell: {:?}",
                entries.iter().map(|e| e.label.clone()).collect::<Vec<_>>()
            );
            table.run_action(CommandId::POD_SHELL, vec![target.clone()], window, cx);
        });
    });

    // A terminal tab in the cluster tab's bottom dock, running the pod's shell.
    let inner = vcx.update(|_, cx| tab.read(cx).workspace().clone());
    let terminal = wait_value(&mut vcx, "the pod terminal to open", |vcx| {
        vcx.update(|_, cx| {
            inner
                .read(cx)
                .items_of_type::<TerminalView>()
                .first()
                .cloned()
        })
    });
    let (docked, panel) = vcx.update(|_, cx| {
        let ws = inner.read(cx);
        (
            ws.item_dock(terminal.entity_id(), cx),
            ws.panel::<TerminalPanel>().is_some(),
        )
    });
    assert_eq!(docked, Some(DockPosition::Bottom));
    assert!(panel);
    let descriptor = vcx.update(|_, cx| terminal.read(cx).descriptor().clone());
    let BackendDescriptor::Exec {
        pod,
        container,
        command,
    } = descriptor
    else {
        panic!("a pod shell");
    };
    assert_eq!(pod, target);
    assert_eq!(
        container.as_deref(),
        Some("main"),
        "the pod's only container, chosen first"
    );
    assert!(command.is_empty(), "the shell chain, not a fixed program");

    // The notice line names the shell that opened: busybox has no bash.
    wait(
        &mut vcx,
        "the session to open and announce the shell",
        |vcx| screen(vcx, &terminal).contains("bash not found, using sh in shell-target/main"),
    );
    let state = vcx
        .update(|_, cx| terminal.read(cx).terminal().cloned())
        .expect("running");
    vcx.update(|_, cx| state.read(cx).input("echo who=$HOSTNAME sum=$((40+2))\n"));
    wait(&mut vcx, "the command's output on screen", |vcx| {
        let text = screen(vcx, &terminal);
        text.contains("who=shell-target sum=42")
    });

    // The open was audited by the guard: initiator, the pod, the container, never the content.
    let app = vcx.update(|_, cx| AppState::global(cx));
    let records = futures::executor::block_on(app.state().query_audit(&AuditQuery::default()))
        .expect("the audit log");
    let shells: Vec<_> = records.iter().filter(|r| &*r.cmd == "pod::Shell").collect();
    assert_eq!(shells.len(), 1, "{records:?}");
    assert_eq!(shells[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(shells[0].initiator, Initiator::Ui);
    assert_eq!(shells[0].target, target);
    assert_eq!(
        shells[0].detail.as_deref(),
        Some("session=shell container=main"),
        "the container the picker (or the only choice) resolved"
    );
    let json = serde_json::to_string(&records).unwrap();
    assert!(
        !json.contains("sum=42"),
        "typed input and output are never recorded"
    );

    // Close the tab (ends the session), disconnect so the liveness loop stops, and let go.
    vcx.update(|window, cx| {
        let id = terminal.entity_id();
        inner.update(cx, |ws, cx| ws.close_item(id, window, cx));
    });
    app.services().sessions.disconnect(&cluster).ok();
    vcx.run_until_parked();
}

#[gpui::test]
fn a_debug_container_is_added_from_the_pods_row_and_opens_a_terminal_in_it(
    cx: &mut TestAppContext,
) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    cx.executor().allow_parking();

    let ns = TestNamespace::create(&context).expect("namespace");
    create_pod(&context, ns.name(), "debug-target");
    let Launched {
        mut vcx,
        tab,
        cluster,
        table,
        _dir,
    } = launch(cx, &context);
    let target = ResourceRef::namespaced(
        cluster.clone(),
        Gvk::new("", "v1", "Pod"),
        ns.name(),
        "debug-target",
    );

    // "Debug" is offered on the pod's row, enabled on this (writable) cluster; choosing it opens the
    // dialog in the cluster tab, which reads the pod for its containers first.
    let inner = vcx.update(|_, cx| tab.read(cx).workspace().clone());
    vcx.update(|window, cx| {
        table.update(cx, |table, cx| {
            let entries = table.action_entries(cx);
            assert!(
                entries
                    .iter()
                    .any(|e| e.command() == CommandId::POD_DEBUG && e.is_enabled()),
                "the pods table offers Debug: {:?}",
                entries.iter().map(|e| e.label.clone()).collect::<Vec<_>>()
            );
            table.run_action(CommandId::POD_DEBUG, vec![target.clone()], window, cx);
        });
    });
    let dialog = wait_value(&mut vcx, "the debug dialog to open", |vcx| {
        vcx.update(|_, cx| {
            let layer = inner.read(cx).modal_layer().clone();
            layer.read(cx).active_modal::<DebugDialog>()
        })
    });
    let (image, command, target_name) = vcx.update(|_, cx| {
        let defaults = dialog.read(cx).defaults();
        (
            defaults.image.clone(),
            defaults.command.clone(),
            defaults.targets[defaults.target].name.to_string(),
        )
    });
    assert_eq!((image.as_str(), command.as_str()), ("busybox", "sh"));
    assert_eq!(target_name, "main", "the pod's default container");

    // The suites share one pinned image (the dialog's own default is the unpinned `busybox`).
    vcx.update(|window, cx| {
        dialog.update(cx, |dialog, cx| {
            dialog.fill(BUSYBOX, "sh", "", window, cx);
            dialog.submit(cx);
        });
    });

    // A terminal tab attached to the new container opens in the bottom dock once it runs.
    let terminal = wait_value(&mut vcx, "the debug terminal to open", |vcx| {
        vcx.update(|_, cx| {
            inner
                .read(cx)
                .items_of_type::<TerminalView>()
                .first()
                .cloned()
        })
    });
    let docked = vcx.update(|_, cx| inner.read(cx).item_dock(terminal.entity_id(), cx));
    assert_eq!(docked, Some(DockPosition::Bottom));
    let descriptor = vcx.update(|_, cx| terminal.read(cx).descriptor().clone());
    let BackendDescriptor::Attach {
        pod,
        container: Some(container),
    } = descriptor
    else {
        panic!("a debug terminal is an attach to the new container, got {descriptor:?}");
    };
    assert_eq!(pod, target);
    assert!(container.starts_with("debugger-"), "{container}");
    wait(&mut vcx, "the notice naming the debug container", |vcx| {
        screen(vcx, &terminal).contains("sharing the processes of main")
    });
    let state = vcx
        .update(|_, cx| terminal.read(cx).terminal().cloned())
        .expect("running");
    vcx.update(|_, cx| state.read(cx).input("ps; echo ps-done-$((20+1))\n"));
    wait(&mut vcx, "ps to list the target's process", |vcx| {
        let text = screen(vcx, &terminal);
        text.contains("ps-done-21") && text.contains("sleep 3600")
    });

    // The pod lists the ephemeral container, as asked; the dialog is gone.
    assert_eq!(
        ephemeral_containers(&context, ns.name(), "debug-target"),
        [(container, BUSYBOX.to_owned(), "main".to_owned())]
    );
    let dialog_open = vcx.update(|_, cx| {
        let layer = inner.read(cx).modal_layer().clone();
        layer.read(cx).active_modal::<DebugDialog>().is_some()
    });
    assert!(!dialog_open, "the dialog closed when the container ran");

    // One guarded, audited mutation: the image and the target, never what was typed.
    let app = vcx.update(|_, cx| AppState::global(cx));
    let records = futures::executor::block_on(app.state().query_audit(&AuditQuery::default()))
        .expect("the audit log");
    let debugs: Vec<_> = records.iter().filter(|r| &*r.cmd == "pod::Debug").collect();
    assert_eq!(debugs.len(), 1, "{records:?}");
    assert_eq!(debugs[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(debugs[0].initiator, Initiator::Ui);
    assert_eq!(debugs[0].target, target);
    assert_eq!(
        debugs[0].detail.as_deref(),
        Some(format!("session=debug image={BUSYBOX} target=main program=sh").as_str())
    );
    assert!(
        !serde_json::to_string(&records).unwrap().contains("ps-done"),
        "typed input is never recorded"
    );

    // Closing the tab ends the attach session only: the container stays in the pod.
    vcx.update(|window, cx| {
        let id = terminal.entity_id();
        inner.update(cx, |ws, cx| ws.close_item(id, window, cx));
    });
    vcx.run_until_parked();
    assert_eq!(
        ephemeral_containers(&context, ns.name(), "debug-target").len(),
        1
    );
    app.services().sessions.disconnect(&cluster).ok();
    vcx.run_until_parked();
}
