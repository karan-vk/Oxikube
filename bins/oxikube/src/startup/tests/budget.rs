//! The start-up budget (E05-S13): the first interactive frame is marked with no network and no
//! lazy service started, the main window opens behind the placeholder over the `AppState`'s state
//! db, and settings, theme and keymap load stays inside its main-thread budget.

use std::time::{Duration, Instant};

use gpui::{TestAppContext, VisualTestContext};
use oxikube_runtime::LazyServices;
use oxikube_ui::root::Root;
use oxikube_workspace::persistence::RestoreStatus;
use oxikube_workspace::window::MainView;

use crate::startup::{
    CONFIG_LOAD_BUDGET, ConfigSource, FirstFrame, Stage, StartupEnv, StartupReport, first_frame,
    init, window,
};

/// What the main-thread config load may cost in a test build. The strict 30 ms
/// ([`CONFIG_LOAD_BUDGET`]) is checked by the optimised `cargo xtask perf startup` run; an
/// unoptimised test binary on a loaded CI runner gets five times that.
const GENEROUS_CONFIG_LOAD: Duration = Duration::from_millis(150);

fn start(cx: &mut TestAppContext, env: StartupEnv) -> VisualTestContext {
    cx.update(|cx| init(cx, env)).expect("the init order runs");
    let handle = cx
        .update(|cx| window::open_main_window(cx, |content, _| content))
        .expect("the main window opens");
    let vcx = VisualTestContext::from_window(handle.into(), cx);
    vcx.run_until_parked();
    vcx
}

fn launched_env() -> StartupEnv {
    let mut env = StartupEnv::test();
    env.earlier = StartupReport::launched_at(Instant::now());
    env
}

#[gpui::test]
fn the_first_frame_ends_start_up_with_no_network_and_nothing_lazy_started(cx: &mut TestAppContext) {
    let mut vcx = start(cx, launched_env());
    vcx.update(|_, cx| {
        let report = StartupReport::get(cx).expect("installed by init");
        let frame = report
            .first_frame()
            .expect("the probe marked the first frame");
        assert!(
            frame.since_launch.is_some(),
            "timed from the launch instant"
        );
        if cfg!(any(target_os = "linux", target_os = "macos")) {
            assert_eq!(
                frame.inet_sockets,
                Some(0),
                "no network before the first frame"
            );
        }
        assert!(
            LazyServices::started(cx).is_empty(),
            "no deferred service starts before the first frame"
        );
        let summary = first_frame::summary(report);
        assert!(
            summary.starts_with("first interactive frame after "),
            "{summary}"
        );
        assert!(summary.contains("network sockets"), "{summary}");
        assert!(summary.contains(" settings "), "{summary}");
    });
}

#[gpui::test]
fn the_main_window_restores_through_the_app_state_behind_the_placeholder(cx: &mut TestAppContext) {
    let mut vcx = start(cx, launched_env());
    vcx.update(|window, cx| {
        let main = window
            .root::<Root>()
            .flatten()
            .expect("the Root")
            .read(cx)
            .view()
            .clone();
        // Under the probe: Root -> FirstFrameProbe -> MainView.
        let probe = main
            .downcast::<oxikube_runtime::perf::FirstFrameProbe>()
            .expect("the first-frame probe wraps the content");
        let main = probe
            .read(cx)
            .inner()
            .clone()
            .downcast::<MainView>()
            .expect("the main view");
        let persistence = main
            .read(cx)
            .persistence()
            .expect("the layout is persisted");
        // The fake state db holds nothing: the placeholder stays as the usable default layout.
        assert_eq!(persistence.read(cx).status(), &RestoreStatus::NothingSaved);
        assert!(!main.read(cx).is_restoring(cx));
    });
}

#[test]
fn a_frame_is_marked_once() {
    let mut report = StartupReport::default();
    let first = FirstFrame {
        since_launch: Some(Duration::from_millis(120)),
        inet_sockets: Some(0),
    };
    report.record_first_frame(first);
    report.record_first_frame(FirstFrame {
        since_launch: Some(Duration::from_millis(900)),
        inet_sockets: Some(3),
    });
    assert_eq!(report.first_frame(), Some(first));
}

#[test]
fn config_load_is_the_settings_theme_and_keymap_stages() {
    let mut report = StartupReport::default();
    report.record(Stage::Runtime, Duration::from_millis(5));
    report.record(Stage::Settings, Duration::from_millis(1));
    report.record(Stage::Theme, Duration::from_millis(2));
    report.record(Stage::Keymap, Duration::from_millis(3));
    report.record(Stage::Ui, Duration::from_millis(60));
    assert_eq!(report.config_load(), Duration::from_millis(6));
    assert_eq!(report.elapsed(Stage::Ui), Some(Duration::from_millis(60)));
    assert_eq!(report.elapsed(Stage::Window), None);
}

#[gpui::test]
fn settings_theme_and_keymap_load_stays_inside_its_budget(cx: &mut TestAppContext) {
    // The default fixture: a fresh config directory, so the user files are created and read like
    // on a first launch, over the embedded defaults (no watchers in tests).
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut env = StartupEnv::test();
    env.config = ConfigSource::Dir(dir.path().to_owned());
    cx.update(|cx| init(cx, env)).expect("the init order runs");
    let load = cx.update(|cx| StartupReport::get(cx).expect("report").config_load());
    assert!(
        load < GENEROUS_CONFIG_LOAD,
        "settings + theme + keymap took {load:?} on the main thread (budget {CONFIG_LOAD_BUDGET:?}, \
         {GENEROUS_CONFIG_LOAD:?} allowed in test builds)"
    );
    assert!(
        dir.path().join("settings.json").exists(),
        "read like a first launch"
    );
}
