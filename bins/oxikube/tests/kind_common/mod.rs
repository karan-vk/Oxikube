//! The app under test, shared by the kind suites of `bins/oxikube` (E09-S08, E09-S13): the real
//! init order with its main window headless, a kind context connected, the pods table open, and
//! helpers to make pods and to wait for what the app does with them. `launch` keeps the SQLite
//! state in a temp dir that the test can read back ([`Launched::data_dir`]).
//!
//! Real I/O wakes GPUI tasks from Tokio threads, so a test that uses this allows parking
//! (`cx.executor().allow_parking()`) and waits with [`wait`], which polls with short real-time
//! sleeps.
#![allow(dead_code)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{Entity, TestAppContext, VisualTestContext};
use oxikube::app_state::AppState;
use oxikube::startup::{
    ConfigSource, PortsChoice, RuntimeChoice, StartupEnv, StartupReport, init, window,
};
use oxikube_app::store::FeedState;
use oxikube_catalog_ui::CatalogView;
use oxikube_domain::ids::ContextName;
use oxikube_domain::session::SessionPhase;
use oxikube_kube::kubeconfig::{Strictness, default_kubeconfig_path, load_local_kubeconfig};
use oxikube_kube::{ClientPool, ContextDefinition, PoolConfig};
use oxikube_resources_ui::table::ResourceTable;
use oxikube_terminal::view::TerminalView;
use oxikube_testkit::integration::pods;
use oxikube_workspace::sidebar::SidebarPanel;
use oxikube_workspace::{ClusterTab, Workspace};

/// Far longer than a connect or an exec to kind takes; a hang fails here.
pub const DEADLINE: Duration = Duration::from_secs(60);

/// What a test drives: the app's headless main window with `context` connected, its cluster tab, and
/// the pods table open.
pub struct Launched {
    pub vcx: VisualTestContext,
    pub tab: Entity<ClusterTab>,
    pub cluster: oxikube_domain::ids::ClusterId,
    pub table: Entity<ResourceTable>,
    pub _dir: tempfile::TempDir,
}

impl Launched {
    /// The directory holding the app's SQLite state and settings for this run.
    pub fn data_dir(&self) -> &std::path::Path {
        self._dir.path()
    }
}

/// Starts the app on the real init order (SQLite state in a temp dir, the default kubeconfig as the
/// only source), connects `context` from the catalog and opens the pods table.
pub fn launch(cx: &mut TestAppContext, context: &str) -> Launched {
    launch_with(cx, context, serde_json::json!({}))
}

/// [`launch`] with `extra` merged into the user's `settings.json` (a JSON object of top-level
/// settings, such as `{"terminal": {"shell": "/bin/sh"}}`).
pub fn launch_with(cx: &mut TestAppContext, context: &str, extra: serde_json::Value) -> Launched {
    let dir = tempfile::tempdir().expect("a temp dir");
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("settings.json"), {
        let mut settings =
            serde_json::json!({ "kubeconfig": { "sources": [{ "kind": "default" }] } });
        if let (Some(settings), Some(extra)) = (settings.as_object_mut(), extra.as_object()) {
            settings.extend(extra.clone());
        }
        settings.to_string()
    })
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
pub fn on_cluster<T>(context: &str, call: impl AsyncFnOnce(kube::Client) -> T) -> T {
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

pub fn pods_api(client: kube::Client, namespace: &str) -> kube::Api<kube::api::DynamicObject> {
    let resource = kube::api::ApiResource::from_gvk_with_plural(
        &kube::api::GroupVersionKind::gvk("", "v1", "Pod"),
        "pods",
    );
    kube::Api::namespaced_with(client, namespace, &resource)
}

/// A busybox pod `name` with one container `main` that sleeps, ready to be exec'd into
/// (`oxikube_testkit::integration::pods::sleeper`).
pub fn create_pod(context: &str, namespace: &str, name: &str) {
    create_pod_from(context, namespace, pods::sleeper(name));
}

/// Creates the pod `manifest` in `namespace` and waits until it is `Running`.
pub fn create_pod_from(context: &str, namespace: &str, manifest: serde_json::Value) {
    on_cluster(context, async |client| {
        let pods = pods_api(client, namespace);
        let pod: kube::api::DynamicObject = serde_json::from_value(manifest).expect("a pod");
        let name = pod.metadata.name.clone().expect("a pod name");
        pods.create(&kube::api::PostParams::default(), &pod)
            .await
            .expect("create the pod");
        let started = Instant::now();
        loop {
            let phase = pods
                .get(&name)
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
pub fn ephemeral_containers(
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
pub fn screen(vcx: &mut VisualTestContext, terminal: &Entity<TerminalView>) -> String {
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
pub fn open_pods_table(
    vcx: &mut VisualTestContext,
    tab: &Entity<ClusterTab>,
) -> Entity<ResourceTable> {
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

pub fn wait_ready(vcx: &mut VisualTestContext, context: &str) -> oxikube_domain::ids::ClusterId {
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

pub fn tab_of(
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

pub fn workspace(vcx: &mut VisualTestContext) -> Entity<Workspace> {
    vcx.update(|window, cx| {
        let main = window::main_view(window, cx).expect("the app's main view");
        main.read(cx).workspace().clone()
    })
}

/// Polls until `done`, running the app in between; panics after [`DEADLINE`].
pub fn wait(
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
pub fn wait_value<T>(
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

/// "Shell" on `pod`'s row of the pods table: the terminal tab the command opens in the cluster
/// tab's bottom dock, once it exists. The session behind it may not be open yet; wait for its
/// first line with [`screen`].
pub fn open_pod_shell(
    vcx: &mut VisualTestContext,
    tab: &Entity<ClusterTab>,
    table: &Entity<ResourceTable>,
    pod: &oxikube_domain::ids::ResourceRef,
) -> Entity<TerminalView> {
    vcx.update(|window, cx| {
        table.update(cx, |table, cx| {
            table.run_action(
                oxikube_domain::command::CommandId::POD_SHELL,
                vec![pod.clone()],
                window,
                cx,
            );
        });
    });
    let inner = vcx.update(|_, cx| tab.read(cx).workspace().clone());
    wait_value(vcx, "the pod terminal to open", |vcx| {
        vcx.update(|_, cx| {
            inner
                .read(cx)
                .items_of_type::<TerminalView>()
                .first()
                .cloned()
        })
    })
}

/// Types `text` into the terminal's process, as the keyboard would send it.
pub fn type_into(vcx: &mut VisualTestContext, terminal: &Entity<TerminalView>, text: &str) {
    vcx.update(|_, cx| {
        let state = terminal.read(cx).terminal().cloned().expect("running");
        state.read(cx).input(text.to_owned());
    });
}

/// The grid's size in cells, `(columns, rows)`.
pub fn grid_size(vcx: &mut VisualTestContext, terminal: &Entity<TerminalView>) -> (u16, u16) {
    vcx.update(|_, cx| {
        let state = terminal.read(cx).terminal().cloned().expect("running");
        let size = state.read(cx).size();
        (size.width, size.height)
    })
}
