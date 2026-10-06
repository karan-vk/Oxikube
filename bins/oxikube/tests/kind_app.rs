//! The app against a real cluster (E07-S00): the real init order with the app's own adapters
//! (`PortsChoice::Sqlite`: SQLite state db, the kube catalog and connector, the Tokio runtime),
//! its main window headless (GPUI's test platform, no display), and a user's path through it:
//! the catalog lists the kind context, searching for it and pressing Enter connects it and opens
//! its cluster tab with the sidebar; a context whose token the API server rejects opens a tab
//! that shows `AuthRequired`, not a blank screen.
//!
//! `cargo test -p oxikube --features integration --test kind_app` with `OXIKUBE_TEST_CONTEXT`
//! set (`cargo xtask kind-up`); without it the test returns at once. It creates nothing in the
//! cluster. Its only file outside a temp dir is none: the extra kubeconfig (the kind server, its
//! public CA and a made-up token) lives in a temp dir removed at the end.
//!
//! Real I/O wakes GPUI tasks from Tokio and SQLite threads, so the test allows parking and polls
//! with short real-time sleeps; everything else is the deterministic test scheduler.
#![cfg(feature = "integration")]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::{Entity, TestAppContext, VisualTestContext};
use oxikube::app_state::AppState;
use oxikube::startup::{
    ConfigSource, PortsChoice, RuntimeChoice, StartupEnv, StartupReport, init, window,
};
use oxikube_catalog_ui::{CatalogView, ConnectView};
use oxikube_domain::ids::ContextName;
use oxikube_domain::session::SessionPhase;
use oxikube_kube::kubeconfig::{Strictness, default_kubeconfig_path, load_local_kubeconfig};
use oxikube_testkit::integration::{ensure_kind_context, test_context};
use oxikube_ui::root::Root;
use oxikube_workspace::sidebar::SidebarPanel;
use oxikube_workspace::window::MainView;
use oxikube_workspace::{ClusterTab, Workspace};

/// The context whose token the API server rejects.
const BAD: &str = "oxi-app-bad-token";
/// Far longer than a connect to kind takes; a hang fails here.
const DEADLINE: Duration = Duration::from_secs(60);

#[gpui::test]
fn the_app_connects_kind_from_the_catalog_and_shows_a_bad_context_as_auth_required(
    cx: &mut TestAppContext,
) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    cx.executor().allow_parking();

    let dir = tempfile::tempdir().expect("a temp dir");
    let bad = dir.path().join("bad-token.yaml");
    write_bad_token_kubeconfig(&context, &bad);
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    let settings = serde_json::json!({
        "kubeconfig": { "sources": [
            { "kind": "default" },
            { "kind": "file", "path": bad.display().to_string() },
        ] }
    });
    std::fs::write(config.join("settings.json"), settings.to_string()).unwrap();

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

    // The catalog reads the user's kubeconfig and the extra file, after the first frame.
    wait(&mut vcx, "the catalog lists both contexts", |vcx| {
        vcx.update(|_, cx| {
            let model = catalog.read(cx).model();
            let names: Vec<String> = (0..model.visible_len())
                .filter_map(|ix| model.row(ix))
                .map(|row| row.entry().context.context.to_string())
                .collect();
            names.contains(&context) && names.iter().any(|n| n == BAD)
        })
    });

    connect_from_catalog(&mut vcx, &catalog, &context);
    let kind = wait_for_phase(&mut vcx, &context, |phase| phase == SessionPhase::Ready);
    let tab = tab_of(&mut vcx, &workspace, &kind);
    let sidebar = vcx.update(|_, cx| {
        tab.read(cx)
            .workspace()
            .read(cx)
            .panel::<SidebarPanel>()
            .is_some()
    });
    assert!(sidebar, "the kind tab has its sidebar");

    // Back to the catalog (the first tab) for the second connect.
    let catalog_id = catalog.entity_id();
    vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| ws.activate_item(catalog_id, true, window, cx));
    });
    connect_from_catalog(&mut vcx, &catalog, BAD);
    let bad = wait_for_phase(&mut vcx, BAD, |phase| {
        matches!(phase, SessionPhase::AuthRequired | SessionPhase::Error)
    });
    let tab = tab_of(&mut vcx, &workspace, &bad);
    let connect_view = vcx.update(|_, cx| {
        tab.read(cx)
            .connect_ui()
            .is_some_and(|ui| ui.body.clone().downcast::<ConnectView>().is_ok())
    });
    assert!(
        connect_view,
        "the failed cluster's tab shows the connect view"
    );
    let state = vcx.update(|_, cx| AppState::global(cx));
    let reason = format!("{:?}", state.services().sessions.get(&bad).unwrap().state());
    assert!(
        !reason.contains("not-a-valid-token"),
        "token leaked: {reason}"
    );

    // Disconnect so the liveness loop stops before the runtime goes.
    for cluster in [kind, bad] {
        state.services().sessions.disconnect(&cluster).ok();
    }
    vcx.run_until_parked();
}

/// Types `name` into the catalog's search and presses Enter, as a user would.
fn connect_from_catalog(vcx: &mut VisualTestContext, catalog: &Entity<CatalogView>, name: &str) {
    vcx.update(|window, cx| {
        catalog.update(cx, |view, cx| {
            view.set_search(name, window, cx);
            view.focus_search(window, cx);
        });
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("enter");
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

/// Waits until the session of `context` is in a phase `accept` takes; returns its cluster id.
fn wait_for_phase(
    vcx: &mut VisualTestContext,
    context: &str,
    accept: impl Fn(SessionPhase) -> bool,
) -> oxikube_domain::ids::ClusterId {
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
            Some(s) if accept(s.phase()) => {
                eprintln!("{context}: {:?}", s.phase());
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
        let probe = window
            .root::<Root>()
            .flatten()
            .expect("the Root")
            .read(cx)
            .view()
            .clone()
            .downcast::<oxikube_runtime::perf::FirstFrameProbe>()
            .expect("the first-frame probe");
        let main = probe
            .read(cx)
            .inner()
            .clone()
            .downcast::<MainView>()
            .expect("the main view");
        main.read(cx).workspace().clone()
    })
}

/// Writes a kubeconfig with one context, [`BAD`], on the kind cluster's server (its public CA
/// included) whose user has a token no API server accepts. Not a credential.
fn write_bad_token_kubeconfig(context: &str, path: &Path) {
    let home = std::env::var_os("HOME").map(|h| default_kubeconfig_path(&PathBuf::from(h)));
    // The loader reads files on Tokio's blocking pool: a runtime of its own, outside GPUI.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let loaded = runtime
        .block_on(load_local_kubeconfig(
            std::env::var_os("KUBECONFIG"),
            home,
            Strictness::RequireUsable,
        ))
        .expect("load the local kubeconfig");
    let merged = &loaded.merged;
    let cluster_name = merged
        .contexts
        .iter()
        .find(|c| c.name == context)
        .and_then(|c| c.context.as_ref())
        .map(|c| c.cluster.clone())
        .expect("the kind context");
    let cluster = merged
        .clusters
        .iter()
        .find(|c| c.name == cluster_name)
        .and_then(|c| c.cluster.as_ref())
        .expect("the kind cluster");
    let config = serde_json::json!({
        "apiVersion": "v1",
        "kind": "Config",
        "clusters": [{ "name": "kind-bad", "cluster": {
            "server": cluster.server,
            "certificate-authority-data": cluster.certificate_authority_data,
        } }],
        "users": [{ "name": "bad", "user": { "token": "not-a-valid-token" } }],
        "contexts": [{ "name": BAD, "context": { "cluster": "kind-bad", "user": "bad" } }],
    });
    std::fs::write(path, config.to_string()).expect("write the extra kubeconfig");
}
