//! `#[gpui::test]`s of the selector over `oxikube_testkit` fakes: a real `ClusterSessionManager`
//! and `NamespaceService` on in-memory ports, the deterministic runtime bridge, no threads.
//!
//! The service waits on the fake clock (`Env::clock`) for its debounce, GPUI's own clock is
//! separate: `settle` advances the fake one and runs what it wakes.

mod dropdown;
mod keys;
mod model;
mod restricted;
mod session;

use std::sync::Arc;
use std::time::Duration;

use gpui::{TestAppContext, point};
use oxikube_app::ClusterSessionManager;
use oxikube_app::session::namespaces::{NamespacePrefs, NamespaceService, prefs_key};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::{ClusterContext, SourceId, StatePort as _};
use oxikube_testkit::gpui_test::{TestApp, TestWindow};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterPorts, FakeClusterSourcePort, FakeStatePort,
};
use serde_json::json;

use super::NamespaceSelector;

/// One connected cluster on fakes, and the service over it.
pub(super) struct Env {
    pub manager: ClusterSessionManager,
    pub ports: FakeClusterPorts,
    pub state: Arc<FakeStatePort>,
    pub clock: Arc<FakeClockPort>,
    pub service: NamespaceService,
    pub cluster: ClusterId,
}

pub(super) fn namespace(name: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "Namespace",
        "metadata": { "name": name },
    }))
    .expect("namespace json")
}

impl Env {
    /// A connected cluster whose namespaces are `namespaces`.
    pub fn new(namespaces: &[&str]) -> Self {
        let context = ContextName::new("kind-oxikube");
        let cluster = ClusterId::new("/home/me/.kube/config", &context);
        let entry = ClusterContext {
            cluster: cluster.clone(),
            context,
            source: SourceId("kubeconfig".into()),
            server: None,
            default_namespace: None,
        };
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let ports = connector.ports_for(&cluster);
        for name in namespaces {
            ports.resources.insert(namespace(name));
        }
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry.clone()]));
        let clock = Arc::new(FakeClockPort::default());
        let manager = ClusterSessionManager::new(connector, source, clock.clone());
        manager.open(&entry, Default::default());
        futures::executor::block_on(manager.connect(&cluster)).expect("connect");
        let state = Arc::new(FakeStatePort::new());
        let service = NamespaceService::new(manager.clone(), state.clone(), clock.clone());
        Self {
            manager,
            ports,
            state,
            clock,
            service,
            cluster,
        }
    }

    /// Stores prefs as an earlier run would have.
    pub fn remember(&self, prefs: &NamespacePrefs) {
        futures::executor::block_on(self.state.kv_set(
            &prefs_key(&self.cluster),
            serde_json::to_value(prefs).unwrap(),
        ))
        .unwrap();
    }

    /// What is stored for the cluster.
    pub fn stored(&self) -> NamespacePrefs {
        let value = futures::executor::block_on(self.state.kv_get(&prefs_key(&self.cluster)))
            .unwrap()
            .expect("something was stored");
        serde_json::from_value(value).unwrap()
    }

    /// The session's selection.
    pub fn session_selection(&self) -> NamespaceSelection {
        self.manager
            .get(&self.cluster)
            .expect("session")
            .namespace_selection()
            .clone()
    }

    /// Lets the debounce (150 ms on the service's clock) elapse and runs what it wakes.
    pub fn settle(&self, window: &TestWindow<NamespaceSelector>) {
        self.clock.advance(Duration::from_millis(150));
        window.run_until_parked();
    }
}

pub(super) fn prefs(selection: &[&str], favourites: &[&str]) -> NamespacePrefs {
    NamespacePrefs {
        selection: NamespaceSelection::from_names(selection),
        favourites: favourites.iter().copied().collect(),
        typed: Vec::new(),
    }
}

/// Sets the app up (ui globals, deterministic runtime, the selector's keys) and opens a window
/// whose root is a selector for `env`'s cluster.
pub(super) fn open(cx: &mut TestAppContext, env: &Env) -> TestWindow<NamespaceSelector> {
    open_with(cx, env, env.service.clone())
}

/// The globals a selector needs: ui, the deterministic runtime bridge, the selector's keys.
pub(super) fn init_app(cx: &mut TestAppContext) {
    cx.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_runtime::init_deterministic(cx);
        cx.set_reduce_motion(true);
        crate::init(cx);
    });
}

/// Like [`open`], over `service` (a second service over the same stored state looks like a new
/// run of the app).
pub(super) fn open_with(
    cx: &mut TestAppContext,
    env: &Env,
    service: NamespaceService,
) -> TestWindow<NamespaceSelector> {
    init_app(cx);
    let cluster = env.cluster.clone();
    let mut app = TestApp::new(cx);
    let mut window =
        app.open_window(move |window, cx| NamespaceSelector::new(cluster, service, window, cx));
    window.draw_frame();
    window
}

/// Clicks the element tagged `selector`.
pub(super) fn click(window: &mut TestWindow<NamespaceSelector>, selector: &'static str) {
    window.draw_frame();
    let bounds = window
        .bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not on screen"));
    window.simulate_click(
        point(bounds.center().x, bounds.center().y),
        gpui::Modifiers::none(),
    );
    window.draw_frame();
}

/// Opens the dropdown with a click on the trigger.
pub(super) fn open_dropdown(window: &mut TestWindow<NamespaceSelector>) {
    click(window, "namespace-trigger");
    assert!(window.read_root(|s, _| s.is_open()), "the dropdown opened");
}

/// The selections of the `NamespaceChanged` updates sent so far.
pub(super) fn namespace_changes(
    updates: &mut oxikube_app::SessionUpdates,
) -> Vec<NamespaceSelection> {
    use futures::{FutureExt as _, StreamExt as _};
    let mut out = Vec::new();
    while let Some(Some(Ok(update))) = updates.next().now_or_never() {
        if let oxikube_app::SessionChange::NamespaceChanged(selection) = update.change {
            out.push(selection);
        }
    }
    out
}
