//! The documented order, the globals it leaves behind, and the refusal to run twice.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use gpui::{App, BorrowAppContext as _, TestAppContext};
use oxikube_keymap::KeymapStore;
use oxikube_logging::{LogConfig, LogSettings, build};
use oxikube_runtime::RuntimeMode;
use oxikube_settings::{Settings as _, SettingsStore};
use oxikube_theme::{ActiveTheme, ThemeRegistry};
use oxikube_workspace::session::SessionSettings;

use crate::app_state::{AppPorts, AppState, AppStateError};
use crate::startup::{
    ConfigSource, Feature, Stage, StartupEnv, StartupError, StartupReport, init, init_with_features,
};

/// The stages `init` runs, as documented in the table of `startup`'s module docs.
const INIT_STAGES: [Stage; 10] = [
    Stage::Runtime,
    Stage::Settings,
    Stage::Theme,
    Stage::Keymap,
    Stage::Ui,
    Stage::StateDb,
    Stage::AppState,
    Stage::Workspace,
    Stage::Features,
    Stage::KeymapRebind,
];

#[gpui::test]
fn the_stages_run_in_the_documented_order(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init(cx, StartupEnv::test()).expect("init order runs");
        let report = StartupReport::get(cx).expect("report installed");
        assert_eq!(report.order(), INIT_STAGES);
        // The documented order is the declaration order of `Stage` (minus the three stages that
        // run in `main`).
        let declared: Vec<Stage> = Stage::ALL
            .into_iter()
            .filter(|s| INIT_STAGES.contains(s))
            .collect();
        assert_eq!(report.order(), declared);
        assert!(report.total() > Duration::ZERO);
    });
}

#[gpui::test]
fn earlier_stages_keep_their_place_in_the_report(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut env = StartupEnv::test();
        env.earlier
            .record(Stage::Logging, Duration::from_micros(40));
        env.earlier.record(Stage::Assets, Duration::from_micros(10));
        init(cx, env).unwrap();
        let order = StartupReport::get(cx).unwrap().order();
        assert_eq!(&order[..2], [Stage::Logging, Stage::Assets]);
        assert_eq!(order.len(), 2 + INIT_STAGES.len());
        // The window is opened after `init`; its cost joins the report.
        crate::startup::time_after_init(cx, Stage::Window, |_| ());
        assert_eq!(
            StartupReport::get(cx).unwrap().order().last(),
            Some(&Stage::Window)
        );
    });
}

#[gpui::test]
fn init_registers_the_expected_globals(cx: &mut TestAppContext) {
    cx.update(|cx| {
        assert!(AppState::try_global(cx).is_none());
        init(cx, StartupEnv::test()).unwrap();

        assert!(cx.has_global::<SettingsStore>());
        assert!(cx.has_global::<ThemeRegistry>());
        assert!(cx.has_global::<ActiveTheme>());
        assert!(cx.has_global::<KeymapStore>());
        assert_eq!(oxikube_runtime::mode(cx), Some(RuntimeMode::Deterministic));

        let state = AppState::global(cx);
        assert_eq!(state.runtime_mode(cx), Some(RuntimeMode::Deterministic));
        assert!(state.runtime_handle(cx).is_none());
        assert!(state.ports().secrets.is_some());
        assert!(state.data_dir().is_none());
        assert!(!state.theme_registry(cx).list().is_empty());
        assert!(state.keymap(cx).diagnostics().is_empty());
        // Typed settings registered by crates that were initialised: workspace session, log.
        assert!(SessionSettings::try_get(cx).is_some());
        assert!(state.settings(cx).try_get::<LogSettings>(None).is_some());
        let _ = state.active_theme(cx);
    });
}

#[gpui::test]
fn a_second_init_is_rejected_and_changes_nothing(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init(cx, StartupEnv::test()).unwrap();
        let first = AppState::global(cx);
        let stages = StartupReport::get(cx).unwrap().timings().len();

        let again = init(cx, StartupEnv::test());
        assert!(
            matches!(again, Err(StartupError::AlreadyInitialised)),
            "{again:?}"
        );

        assert!(
            std::sync::Arc::ptr_eq(&first, &AppState::global(cx)),
            "the first state stays"
        );
        assert_eq!(StartupReport::get(cx).unwrap().timings().len(), stages);
    });
}

#[gpui::test]
fn app_state_test_runs_the_real_order_once(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let first = AppState::test(cx);
        let second = AppState::test(cx);
        assert!(std::sync::Arc::ptr_eq(&first, &second));
        assert_eq!(StartupReport::get(cx).unwrap().order(), INIT_STAGES);
    });
}

fn fake_ports() -> AppPorts {
    AppPorts::new(std::sync::Arc::new(oxikube_testkit::FakeStatePort::new()))
}

#[gpui::test]
fn install_refuses_until_its_prerequisites_exist(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let install = |cx: &mut App| AppState::new(fake_ports(), None).install(cx);
        assert_eq!(
            install(cx).err(),
            Some(AppStateError::NotInitialised("runtime"))
        );

        oxikube_runtime::init_deterministic(cx);
        assert_eq!(
            install(cx).err(),
            Some(AppStateError::NotInitialised("settings store"))
        );

        cx.set_global(SettingsStore::new(oxikube_assets::default_settings()).unwrap());
        assert_eq!(
            install(cx).err(),
            Some(AppStateError::NotInitialised("theme registry"))
        );

        oxikube_theme::init_with_dir(None, cx);
        assert_eq!(
            install(cx).err(),
            Some(AppStateError::NotInitialised("keymap"))
        );

        oxikube_keymap::init_with_text("", Default::default(), cx);
        let installed = install(cx).expect("every prerequisite exists now");
        assert_eq!(install(cx).err(), Some(AppStateError::AlreadyInstalled));
        assert!(std::sync::Arc::ptr_eq(&installed, &AppState::global(cx)));
    });
}

static ORDER: AtomicUsize = AtomicUsize::new(0);
static FIRST_AT: AtomicUsize = AtomicUsize::new(usize::MAX);
static SECOND_AT: AtomicUsize = AtomicUsize::new(usize::MAX);

fn first(_: &mut App) {
    FIRST_AT.store(ORDER.fetch_add(1, Ordering::SeqCst), Ordering::SeqCst);
}

fn second(_: &mut App) {
    SECOND_AT.store(ORDER.fetch_add(1, Ordering::SeqCst), Ordering::SeqCst);
}

#[gpui::test]
fn feature_crates_init_in_list_order_after_the_workspace(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let features = [
            Feature {
                name: "first",
                init: first,
            },
            Feature {
                name: "second",
                init: second,
            },
        ];
        init_with_features(cx, StartupEnv::test(), &features).unwrap();
        assert_eq!(FIRST_AT.load(Ordering::SeqCst), 0);
        assert_eq!(SECOND_AT.load(Ordering::SeqCst), 1);
        assert_eq!(StartupReport::get(cx).unwrap().order(), INIT_STAGES);
    });
}

#[gpui::test]
fn the_shipped_feature_list_initialises_on_the_test_environment(cx: &mut TestAppContext) {
    // A feature crate added to `FEATURES` must be initialisable without a window or threads.
    cx.update(|cx| {
        init_with_features(cx, StartupEnv::test(), crate::startup::FEATURES).unwrap();
    });
}

#[gpui::test]
fn the_log_filter_follows_the_setting_through_the_state(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = LogConfig::new(dir.path());
    config.honour_rust_log = false;
    let (_dispatch, guard) = build(&config).unwrap();
    let handle = guard.handle();
    let before = handle.directives();

    cx.update(|cx| {
        let mut env = StartupEnv::test();
        env.log = Some(handle.clone());
        init(cx, env).unwrap();
    });
    assert_eq!(before, oxikube_logging::DEFAULT_DIRECTIVES);
    assert_eq!(handle.directives(), before);

    cx.update(|cx| {
        cx.update_global::<SettingsStore, _>(|store, _| {
            store
                .set_user_settings(r#"{ "log": { "filter": "warn,oxikube=debug" } }"#)
                .unwrap();
        });
    });
    assert_eq!(handle.directives(), "warn,oxikube=debug");
}

#[gpui::test]
fn a_config_dir_is_read_once_without_watchers(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("settings.json"),
        r#"{ "ui_scale": 1.5, "log": { "filter": "error" } }"#,
    )
    .unwrap();
    cx.update(|cx| {
        let mut env = StartupEnv::test();
        env.config = ConfigSource::Dir(dir.path().to_owned());
        init(cx, env).unwrap();
        assert_eq!(SessionSettings::get_global(cx).ui_scale.factor(), 1.5);
        assert_eq!(LogSettings::get_global(cx).filter, "error");
        assert!(AppState::global(cx).keymap(cx).diagnostics().is_empty());
    });
}
