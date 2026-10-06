//! Session restore (E06-S11) on the window: placeholder tabs, lazy connects, failures in their
//! own tabs, the dropped-clusters notice and the first-frame rule. The decisions themselves are
//! tested in `oxikube_app::session::restore`; these tests drive the real controller over fakes.

use std::time::Duration;

use gpui::TestAppContext;
use oxikube_app::session::namespaces::NamespaceService;
use oxikube_app::session::restore::{ClusterTabsStore, RestoreConfig, SavedTabs, SessionRestorer};
use oxikube_domain::OxiError;
use oxikube_domain::session::{ClusterSessionState, SessionPhase};
use oxikube_settings::Settings as _;

use super::*;
use crate::cluster_tab::{RestoreConnectSetting, SessionRestoreSettings};
use crate::persistence::{MAIN_WINDOW_ID, SAVE_DEBOUNCE};

const TIMEOUT: Duration = Duration::from_secs(30);

/// A fixture whose state holds a saved session, with `session.restore` set as asked.
struct Restore {
    fx: Fixture,
    restorer: SessionRestorer,
    _config: tempfile::TempDir,
}

impl Restore {
    /// A window over contexts `names` and a saved session `open` with `active` displayed.
    fn new(
        cx: &mut TestAppContext,
        names: &[&str],
        open: &[&str],
        active: Option<&str>,
        restore: bool,
        connect: RestoreConnectSetting,
    ) -> Self {
        let state = Arc::new(FakeStatePort::new());
        let store = ClusterTabsStore::new(state.clone(), MAIN_WINDOW_ID).expect("store");
        let saved = SavedTabs::new(open.iter().map(|n| id(n)).collect(), active.map(id))
            .with_titles(open.iter().map(|n| (id(n), (*n).to_owned())));
        block_on(store.save(&saved)).expect("save");

        let mut fx = Fixture::build(cx, names, state.clone(), true, false);
        let config = tempfile::tempdir().expect("a temp dir");
        fx.vcx.update(|_, cx| {
            oxikube_settings::init_with_dir(config.path(), cx);
            SessionRestoreSettings::override_global(
                SessionRestoreSettings { restore, connect },
                cx,
            );
        });
        let namespaces = NamespaceService::new(fx.sessions.clone(), state, fx.clock.clone());
        let restorer = SessionRestorer::new(
            fx.sessions.clone(),
            namespaces,
            fx.source.clone(),
            store,
            RestoreConfig {
                concurrency: 2,
                connect_timeout: TIMEOUT,
            },
        );
        Self {
            fx,
            restorer,
            _config: config,
        }
    }

    /// Starts the restore, as the window does once it is up.
    fn start(&mut self) {
        let (tabs, restorer) = (self.fx.tabs.clone(), self.restorer.clone());
        self.fx.vcx.update(|window, cx| {
            tabs.update(cx, |tabs, cx| {
                tabs.restore_session(restorer, None, window, cx)
            })
        });
        // The task runs up to the frame it waits for.
        self.fx.vcx.run_until_parked();
    }

    /// Draws a frame, then lets everything that was waiting for it run.
    fn first_frame(&mut self) {
        self.fx.vcx.update(|window, cx| {
            window.draw(cx).clear(cx);
            // Tests have no platform frame loop: deliver the frame the restore waits for.
            window.simulate_next_frame(cx);
        });
        self.fx.vcx.run_until_parked();
    }

    fn draw(&mut self) {
        self.fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn start_and_draw(&mut self) {
        self.start();
        self.first_frame();
    }

    fn connects(&self) -> Vec<String> {
        self.fx
            .connector
            .recorded_calls()
            .into_iter()
            .map(|oxikube_testkit::ConnectorCall::Connect { context, .. }| context.to_string())
            .collect()
    }

    fn state_of(&mut self, name: &str) -> ClusterSessionState {
        self.fx
            .tab(name)
            .update(&mut self.fx.vcx, |tab, _| tab.info().state.clone())
    }

    fn placeholder(&mut self, name: &str) -> bool {
        let (tabs, cluster) = (self.fx.tabs.clone(), id(name));
        self.fx
            .vcx
            .update(|_, cx| tabs.read(cx).is_placeholder(&cluster))
    }

    fn saved(&self) -> Option<SavedTabs> {
        let store = ClusterTabsStore::new(self.fx.state.clone(), MAIN_WINDOW_ID).expect("store");
        block_on(store.load()).expect("load")
    }

    fn toasts(&mut self) -> Vec<String> {
        let ws = self.fx.ws.clone();
        self.fx.vcx.update(|_, cx| {
            ws.read(cx)
                .toast_layer()
                .read(cx)
                .visible()
                .iter()
                .map(|toast| toast.message.to_string())
                .collect()
        })
    }
}

#[gpui::test]
fn restore_is_off_by_default(cx: &mut TestAppContext) {
    // The shipped default: `session.restore` is false.
    assert!(!SessionRestoreSettings::from_content(Default::default()).restore);

    let mut r = Restore::new(
        cx,
        &["a", "b"],
        &["a", "b"],
        Some("a"),
        false,
        RestoreConnectSetting::Active,
    );
    r.start_and_draw();
    r.fx.vcx.executor().advance_clock(TIMEOUT);
    r.fx.vcx.run_until_parked();

    assert!(r.fx.open_names().is_empty(), "no tabs reopen");
    assert!(r.connects().is_empty(), "nothing connects");
    assert!(r.fx.sessions.sessions().is_empty(), "no sessions open");
}

#[gpui::test]
fn nothing_is_restored_before_the_first_frame(cx: &mut TestAppContext) {
    let mut r = Restore::new(
        cx,
        &["a", "b"],
        &["a", "b"],
        Some("a"),
        true,
        RestoreConnectSetting::All,
    );
    r.fx.state.clear_calls();
    r.start();
    // The task is parked on the frame: no tab, no session, no connect, no state read.
    r.fx.vcx.run_until_parked();
    assert!(
        r.fx.state.recorded_calls().is_empty(),
        "the state store is not touched before the first frame"
    );
    assert!(r.fx.open_names().is_empty());
    assert!(r.fx.sessions.sessions().is_empty());
    assert!(r.connects().is_empty());

    r.first_frame();
    assert_eq!(r.fx.open_names(), ["a", "b"]);
}

#[gpui::test]
fn placeholder_tabs_appear_in_order_before_any_connect_completes(cx: &mut TestAppContext) {
    let mut r = Restore::new(
        cx,
        &["a", "b", "c"],
        &["c", "a", "b"],
        Some("a"),
        true,
        RestoreConnectSetting::Active,
    );
    // Every connect hangs: the tabs must already be there.
    r.fx.connector.hold();
    r.start_and_draw();

    assert_eq!(r.fx.open_names(), ["c", "a", "b"], "the saved order");
    assert_eq!(
        r.fx.active_name().as_deref(),
        Some("a"),
        "the saved tab is displayed"
    );
    assert_eq!(r.state_of("a"), ClusterSessionState::Connecting);
    assert_eq!(r.state_of("b"), ClusterSessionState::Disconnected);
    assert_eq!(r.state_of("c"), ClusterSessionState::Disconnected);
    assert!(r.placeholder("b") && r.placeholder("c"));
    assert_eq!(r.connects(), ["a"], "only the displayed cluster connects");
    assert_eq!(r.fx.connector.held(), 1);

    // The placeholder says so on screen.
    r.draw();
    let shown = r.fx.vcx.debug_bounds("cluster-placeholder-a").is_some();
    assert!(shown, "the displayed tab draws its connecting placeholder");

    r.fx.connector.release();
    r.fx.vcx.run_until_parked();
    assert_eq!(r.state_of("a"), ClusterSessionState::Ready);
    assert!(!r.placeholder("a"), "a connected tab is an ordinary tab");
    assert_eq!(
        r.connects(),
        ["a"],
        "the others still wait for their tab to be shown"
    );
}

#[gpui::test]
fn a_placeholder_connects_when_its_tab_is_first_shown(cx: &mut TestAppContext) {
    let mut r = Restore::new(
        cx,
        &["a", "b", "c"],
        &["a", "b", "c"],
        Some("a"),
        true,
        RestoreConnectSetting::Active,
    );
    r.start_and_draw();
    assert_eq!(r.connects(), ["a"]);

    assert!(r.fx.apply(Command::ClusterSelect { cluster: id("c") }));

    assert_eq!(r.connects(), ["a", "c"], "showing c connects c, and only c");
    assert_eq!(r.state_of("c"), ClusterSessionState::Ready);
    assert_eq!(r.state_of("b"), ClusterSessionState::Disconnected);
    assert!(r.placeholder("b"));
    assert!(
        r.fx.recorder
            .sent()
            .iter()
            .any(|c| matches!(c, Command::ClusterConnect { cluster } if *cluster == id("c"))),
        "through the same command the catalog sends"
    );
}

#[gpui::test]
fn connect_all_connects_every_restored_cluster(cx: &mut TestAppContext) {
    let mut r = Restore::new(
        cx,
        &["a", "b", "c"],
        &["a", "b", "c"],
        Some("b"),
        true,
        RestoreConnectSetting::All,
    );
    r.start_and_draw();

    assert_eq!(
        r.connects().first().map(String::as_str),
        Some("b"),
        "the displayed one first"
    );
    assert_eq!(r.connects().len(), 3);
    for name in ["a", "b", "c"] {
        assert_eq!(r.state_of(name), ClusterSessionState::Ready, "{name}");
        assert!(!r.placeholder(name));
    }
    assert_eq!(r.fx.active_name().as_deref(), Some("b"));
}

#[gpui::test]
fn a_failed_cluster_shows_its_error_in_its_own_tab_and_the_others_connect(cx: &mut TestAppContext) {
    let mut r = Restore::new(
        cx,
        &["a", "b", "c"],
        &["a", "b", "c"],
        Some("a"),
        true,
        RestoreConnectSetting::All,
    );
    r.fx.connector
        .connect_script_for(&id("a"))
        .push_err(OxiError::unsupported(
            "the API server refused the handshake",
        ));
    r.start_and_draw();

    match r.state_of("a") {
        ClusterSessionState::Error { reason } => assert!(reason.contains("handshake"), "{reason}"),
        other => panic!("a shows its failure, got {other:?}"),
    }
    assert_eq!(r.state_of("b"), ClusterSessionState::Ready);
    assert_eq!(r.state_of("c"), ClusterSessionState::Ready);
    assert_eq!(r.fx.open_names(), ["a", "b", "c"], "the failed tab stays");
    r.draw();
    assert!(
        r.fx.vcx.debug_bounds("cluster-placeholder-a").is_some(),
        "the failed tab draws its error state"
    );
}

#[gpui::test]
fn a_cluster_that_never_answers_ends_in_error_after_the_timeout(cx: &mut TestAppContext) {
    let mut r = Restore::new(
        cx,
        &["a", "b"],
        &["a", "b"],
        Some("a"),
        true,
        RestoreConnectSetting::Active,
    );
    r.fx.connector.hold();
    r.start_and_draw();
    assert_eq!(r.state_of("a"), ClusterSessionState::Connecting);

    r.fx.clock.advance(TIMEOUT);
    r.fx.vcx.run_until_parked();

    match r.state_of("a") {
        ClusterSessionState::Error { reason } => {
            assert!(reason.contains("did not answer"), "{reason}")
        }
        other => panic!("a timed out, got {other:?}"),
    }
    assert_eq!(
        r.fx.open_names(),
        ["a", "b"],
        "the tab stays, showing the error"
    );
}

#[gpui::test]
fn clusters_that_no_longer_exist_are_dropped_with_a_notice(cx: &mut TestAppContext) {
    let mut r = Restore::new(
        cx,
        &["a", "b"],
        &["a", "gone", "b"],
        Some("b"),
        true,
        RestoreConnectSetting::Active,
    );
    r.start_and_draw();

    assert_eq!(r.fx.open_names(), ["a", "b"]);
    let toasts = r.toasts();
    assert_eq!(toasts.len(), 1, "{toasts:?}");
    assert!(toasts[0].contains("gone"), "{toasts:?}");
    assert_eq!(
        r.saved().expect("saved").open,
        [id("a"), id("b")],
        "forgotten, so the next launch is quiet"
    );
}

#[gpui::test]
fn restored_tabs_are_saved_as_they_were(cx: &mut TestAppContext) {
    let mut r = Restore::new(
        cx,
        &["a", "b", "c"],
        &["c", "a", "b"],
        Some("a"),
        true,
        RestoreConnectSetting::Active,
    );
    r.start_and_draw();
    r.fx.vcx
        .executor()
        .advance_clock(SAVE_DEBOUNCE + Duration::from_millis(50));
    r.fx.vcx.run_until_parked();

    let saved = r.saved().expect("saved");
    assert_eq!(saved.open, [id("c"), id("a"), id("b")]);
    assert_eq!(saved.active, Some(id("a")));
    assert_eq!(
        saved.title(&id("a")),
        Some("a"),
        "the names the tabs show are kept"
    );
}

#[gpui::test]
fn closing_a_placeholder_does_not_disconnect_anything(cx: &mut TestAppContext) {
    let mut r = Restore::new(
        cx,
        &["a", "b"],
        &["a", "b"],
        Some("a"),
        true,
        RestoreConnectSetting::Active,
    );
    r.start_and_draw();
    assert!(r.placeholder("b"));

    assert!(r.fx.apply(Command::ClusterCloseTab { cluster: id("b") }));

    assert_eq!(r.fx.open_names(), ["a"]);
    assert_eq!(r.fx.recorder.disconnects(), 0, "nothing was connected");
    assert!(!r.placeholder("b"));
    assert_eq!(
        r.fx.sessions.get(&id("b")).map(|s| s.phase()),
        Some(SessionPhase::Disconnected),
        "the session stays open in the catalog"
    );
}

#[gpui::test]
fn an_unreadable_saved_session_leaves_the_window_as_it_is(cx: &mut TestAppContext) {
    let mut r = Restore::new(
        cx,
        &["a"],
        &["a"],
        Some("a"),
        true,
        RestoreConnectSetting::Active,
    );
    r.fx.state
        .script()
        .table_get
        .push_err(OxiError::internal("the state database is locked"));
    r.start_and_draw();

    assert!(r.fx.open_names().is_empty());
    assert!(r.connects().is_empty());
}

#[gpui::test]
fn the_session_settings_default_to_off_and_follow_settings_json(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().expect("a temp dir");
    cx.update(|cx| oxikube_settings::init_with_dir(dir.path(), cx));
    let read = |cx: &mut TestAppContext| cx.update(|cx| *SessionRestoreSettings::get_global(cx));
    // The shipped default.json: off, and only the displayed cluster would connect.
    assert_eq!(
        read(cx),
        SessionRestoreSettings {
            restore: false,
            connect: RestoreConnectSetting::Active
        }
    );

    // Editing settings.json applies through hot reload.
    cx.update(|cx| {
        oxikube_settings::update_user_settings::<SessionRestoreSettings>(cx, None, |content| {
            content.restore = Some(true);
            content.restore_connect = Some(RestoreConnectSetting::All);
        })
        .detach();
    });
    cx.run_until_parked();
    assert_eq!(
        read(cx),
        SessionRestoreSettings {
            restore: true,
            connect: RestoreConnectSetting::All
        }
    );
    let text = std::fs::read_to_string(dir.path().join("settings.json")).expect("written");
    assert!(
        text.contains("\"session\"") && text.contains("\"restore_connect\""),
        "{text}"
    );
}
