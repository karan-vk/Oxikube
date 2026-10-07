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

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{Entity, TestAppContext, VisualTestContext};
use oxikube::app_state::AppState;
use oxikube::startup::{
    ConfigSource, PortsChoice, RuntimeChoice, StartupEnv, StartupReport, init, window,
};
use oxikube_app::store::FeedState;
use oxikube_catalog_ui::CatalogView;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{ContextName, Gvk, ResourceRef};
use oxikube_domain::session::SessionPhase;
use oxikube_kube::kubeconfig::{Strictness, default_kubeconfig_path, load_local_kubeconfig};
use oxikube_kube::{ClientPool, ContextDefinition, PoolConfig};
use oxikube_ports::AuditQuery;
use oxikube_resources_ui::exec::DebugDialog;
use oxikube_resources_ui::table::ResourceTable;
use oxikube_terminal::view::{BackendDescriptor, TerminalPanel, TerminalView};
use oxikube_testkit::images::BUSYBOX;
use oxikube_testkit::integration::{TestNamespace, ensure_kind_context, test_context};
use oxikube_workspace::sidebar::SidebarPanel;
use oxikube_workspace::{ClusterTab, DockPosition, Workspace};

/// Far longer than a connect or an exec to kind takes; a hang fails here.
const DEADLINE: Duration = Duration::from_secs(60);

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

/// What a test drives: the app's headless main window with `context` connected, its cluster tab, and
/// the pods table open.
struct Launched {
    vcx: VisualTestContext,
    tab: Entity<ClusterTab>,
    cluster: oxikube_domain::ids::ClusterId,
    table: Entity<ResourceTable>,
    _dir: tempfile::TempDir,
}

/// Starts the app on the real init order (SQLite state in a temp dir, the default kubeconfig as the
/// only source), connects `context` from the catalog and opens the pods table.
fn launch(cx: &mut TestAppContext, context: &str) -> Launched {
    let dir = tempfile::tempdir().expect("a temp dir");
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("settings.json"),
        serde_json::json!({ "kubeconfig": { "sources": [{ "kind": "default" }] } }).to_string(),
    )
    .unwrap();
    let env = StartupEnv {
        config: ConfigSource::Dir(config),
        runtime: RuntimeChoice::Tokio,
        ports: PortsChoice::Sqlite(dir.path().join("state.db")),
        data_dir: None,
        log: None,
        earlier: StartupReport::default(),
    };
    cx.update(|cx| init(cx, env)).expect("the init order runs");
    let handle = cx
        .update(|cx| window::open_main_window(cx, |content, _| content))
        .expect("the main window opens");
    let mut vcx = VisualTestContext::from_window(handle.into(), cx);
    let workspace = workspace(&mut vcx);
    let catalog = vcx.update(|_, cx| workspace.read(cx).items_of_type::<CatalogView>()[0].clone());
    wait(&mut vcx, "the catalog lists the kind context", |vcx| {
        vcx.update(|_, cx| {
            let model = catalog.read(cx).model();
            (0..model.visible_len())
                .filter_map(|ix| model.row(ix))
                .any(|row| row.entry().context.context.as_str() == context)
        })
    });
    vcx.update(|window, cx| {
        catalog.update(cx, |view, cx| {
            view.set_search(context, window, cx);
            view.focus_search(window, cx);
        });
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("enter");
    let cluster = wait_ready(&mut vcx, context);
    let tab = tab_of(&mut vcx, &workspace, &cluster);
    let table = open_pods_table(&mut vcx, &tab);
    Launched {
        vcx,
        tab,
        cluster,
        table,
        _dir: dir,
    }
}

/// Runs `call` against a client of the kind `context`, on a runtime of its own (the test's thread
/// is the app's: Tokio work for the setup and the checks must not run on it).
fn on_cluster<T>(context: &str, call: impl AsyncFnOnce(kube::Client) -> T) -> T {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(async {
        let home = std::env::var_os("HOME").map(|h| default_kubeconfig_path(&PathBuf::from(h)));
        let loaded = load_local_kubeconfig(
            std::env::var_os("KUBECONFIG"),
            home,
            Strictness::RequireUsable,
        )
        .await
        .expect("load the local kubeconfig");
        let context = ContextName::new(context);
        let definition = ContextDefinition::from_kubeconfig(&loaded.merged, &context)
            .expect("the kind context is in the kubeconfig");
        let pool = ClientPool::new(definition.kubeconfig().clone(), PoolConfig::default());
        let client = (*pool.get(&context).await.expect("a client")).clone();
        call(client).await
    })
}

fn pods_api(client: kube::Client, namespace: &str) -> kube::Api<kube::api::DynamicObject> {
    let resource = kube::api::ApiResource::from_gvk_with_plural(
        &kube::api::GroupVersionKind::gvk("", "v1", "Pod"),
        "pods",
    );
    kube::Api::namespaced_with(client, namespace, &resource)
}

/// A busybox pod `name` with one container `main` that sleeps, ready to be exec'd into.
fn create_pod(context: &str, namespace: &str, name: &str) {
    on_cluster(context, async |client| {
        let pods = pods_api(client, namespace);
        let pod: kube::api::DynamicObject = serde_json::from_value(serde_json::json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": { "name": name, "labels": { "oxikube.test/suite": "app-exec" } },
            "spec": {
                "restartPolicy": "Never",
                "terminationGracePeriodSeconds": 0,
                "containers": [{ "name": "main", "image": BUSYBOX, "command": ["sleep", "3600"] }],
            },
        }))
        .expect("a pod");
        pods.create(&kube::api::PostParams::default(), &pod)
            .await
            .expect("create the pod");
        let started = Instant::now();
        loop {
            let phase = pods
                .get(name)
                .await
                .ok()
                .and_then(|p| p.data["status"]["phase"].as_str().map(str::to_owned));
            if phase.as_deref() == Some("Running") {
                return;
            }
            assert!(
                started.elapsed() < DEADLINE,
                "the pod did not start: {phase:?}"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
}

/// `(name, image, target)` of the pod's ephemeral containers, from the cluster.
fn ephemeral_containers(
    context: &str,
    namespace: &str,
    name: &str,
) -> Vec<(String, String, String)> {
    on_cluster(context, async |client| {
        let pod = pods_api(client, namespace)
            .get(name)
            .await
            .expect("the pod");
        let text = |value: &serde_json::Value| value.as_str().unwrap_or_default().to_owned();
        pod.data["spec"]["ephemeralContainers"]
            .as_array()
            .map(|containers| {
                containers
                    .iter()
                    .map(|c| {
                        (
                            text(&c["name"]),
                            text(&c["image"]),
                            text(&c["targetContainerName"]),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// The text on the terminal's screen, rows joined by newlines.
fn screen(vcx: &mut VisualTestContext, terminal: &Entity<TerminalView>) -> String {
    vcx.update(|_, cx| {
        let Some(state) = terminal.read(cx).terminal().cloned() else {
            return String::new();
        };
        let snapshot = state.read(cx).snapshot();
        let rows = usize::from(state.read(cx).size().height);
        (0..rows)
            .map(|row| snapshot.row_text(row))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

/// Activates the sidebar's Pods entry and waits for the table to list what the cluster serves.
fn open_pods_table(vcx: &mut VisualTestContext, tab: &Entity<ClusterTab>) -> Entity<ResourceTable> {
    let inner = vcx.update(|_, cx| tab.read(cx).workspace().clone());
    let panel = vcx
        .update(|_, cx| inner.read(cx).panel::<SidebarPanel>())
        .expect("the sidebar");
    wait(vcx, "the sidebar to list Pods", |vcx| {
        vcx.update(|_, cx| panel.read(cx).row("workloads/pods").is_some())
    });
    vcx.update(|_, cx| panel.update(cx, |panel, cx| panel.activate("workloads/pods", cx)));
    wait(vcx, "the pods table to open and list", |vcx| {
        vcx.update(|_, cx| {
            inner
                .read(cx)
                .items_of_type::<ResourceTable>()
                .first()
                .is_some_and(|table| {
                    table.read(cx).read_rows(cx, |d| {
                        d.state() == &FeedState::Ready && !d.rows().is_empty()
                    })
                })
        })
    });
    vcx.update(|_, cx| inner.read(cx).items_of_type::<ResourceTable>()[0].clone())
}

fn wait_ready(vcx: &mut VisualTestContext, context: &str) -> oxikube_domain::ids::ClusterId {
    let context = ContextName::new(context);
    let mut found = None;
    wait(vcx, &format!("{context} to connect"), |vcx| {
        let state = vcx.update(|_, cx| AppState::global(cx));
        let session = state
            .services()
            .sessions
            .sessions()
            .into_iter()
            .find(|s| s.context() == &context);
        match session {
            Some(s) if s.phase() == SessionPhase::Ready => {
                found = Some(s.id().clone());
                true
            }
            _ => false,
        }
    });
    found.expect("found")
}

fn tab_of(
    vcx: &mut VisualTestContext,
    workspace: &Entity<Workspace>,
    cluster: &oxikube_domain::ids::ClusterId,
) -> Entity<ClusterTab> {
    vcx.update(|_, cx| {
        workspace
            .read(cx)
            .items_of_type::<ClusterTab>()
            .into_iter()
            .find(|tab| tab.read(cx).cluster() == cluster)
            .expect("the cluster has a tab")
    })
}

fn workspace(vcx: &mut VisualTestContext) -> Entity<Workspace> {
    vcx.update(|window, cx| {
        let main = window::main_view(window, cx).expect("the app's main view");
        main.read(cx).workspace().clone()
    })
}

/// Polls until `done`, running the app in between; panics after [`DEADLINE`].
fn wait(
    vcx: &mut VisualTestContext,
    what: &str,
    mut done: impl FnMut(&mut VisualTestContext) -> bool,
) {
    let started = Instant::now();
    loop {
        vcx.run_until_parked();
        if done(vcx) {
            return;
        }
        assert!(started.elapsed() < DEADLINE, "timed out waiting: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// [`wait`] for a value.
fn wait_value<T>(
    vcx: &mut VisualTestContext,
    what: &str,
    mut get: impl FnMut(&mut VisualTestContext) -> Option<T>,
) -> T {
    let mut out = None;
    wait(vcx, what, |vcx| {
        out = get(vcx);
        out.is_some()
    });
    out.expect("a value")
}
