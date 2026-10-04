//! GPUI-level tests: the global store, observers, file edits and the reload path.
//!
//! Every test uses `init_with_dir` (no watcher thread), so GPUI's deterministic scheduler
//! sees no foreign threads. The watcher itself is tested with a real file in
//! `src/watcher.rs`.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{AppContext as _, Context, Subscription, TestAppContext, UpdateGlobal as _};
use oxikube_domain::ids::ClusterId;
use oxikube_settings::{
    Settings, SettingsDiagnostic, SettingsLocation, SettingsStore, init_with_dir,
    update_user_settings,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize, JsonSchema)]
struct TerminalContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    font_size: Option<f32>,
}

#[derive(Debug, PartialEq)]
struct TerminalSettings {
    font_size: f32,
}

impl Settings for TerminalSettings {
    const KEY: Option<&'static str> = Some("terminal");
    type Content = TerminalContent;

    fn from_content(content: TerminalContent) -> Self {
        Self {
            font_size: content.font_size.unwrap_or(12.0),
        }
    }
}

#[derive(Default, Serialize, Deserialize, JsonSchema)]
struct GeneralContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ui_scale: Option<f32>,
}

#[derive(Debug, PartialEq)]
struct GeneralSettings {
    ui_scale: f32,
}

impl Settings for GeneralSettings {
    const KEY: Option<&'static str> = None;
    type Content = GeneralContent;

    fn from_content(content: GeneralContent) -> Self {
        Self {
            ui_scale: content.ui_scale.unwrap_or(1.0),
        }
    }
}

const USER: &str = "// mine\n{\n  \"terminal\": {\n    \"font_size\": 13, // keep me\n  },\n}\n";

/// Install the store over a temp config dir holding `user` and register both settings.
fn setup(cx: &mut TestAppContext, user: Option<&str>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    if let Some(user) = user {
        std::fs::write(dir.path().join("settings.json"), user).unwrap();
    }
    cx.update(|cx| {
        init_with_dir(dir.path(), cx);
        TerminalSettings::register(cx);
        GeneralSettings::register(cx);
    });
    dir
}

fn counter() -> (Rc<Cell<usize>>, impl FnMut(&mut gpui::App) + 'static) {
    let count = Rc::new(Cell::new(0));
    let inner = count.clone();
    (count, move |_: &mut gpui::App| inner.set(inner.get() + 1))
}

#[gpui::test]
fn first_run_creates_the_user_file_from_the_template(cx: &mut TestAppContext) {
    let dir = setup(cx, None);
    let path = dir.path().join("settings.json");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        oxikube_assets::initial_user_settings_content()
    );
    cx.read(|cx| {
        let store = cx.global::<SettingsStore>();
        assert_eq!(store.user_settings_path(), Some(path.as_path()));
        assert!(store.diagnostics().is_empty(), "{:?}", store.diagnostics());
        assert_eq!(TerminalSettings::get_global(cx).font_size, 12.0);
    });
}

#[gpui::test]
fn startup_loads_the_existing_user_file(cx: &mut TestAppContext) {
    let _dir = setup(cx, Some(USER));
    cx.read(|cx| assert_eq!(TerminalSettings::get_global(cx).font_size, 13.0));
}

#[gpui::test]
async fn observer_fires_once_on_a_change_and_not_on_an_unrelated_change(cx: &mut TestAppContext) {
    let dir = setup(cx, Some(USER));
    let (terminal_count, on_terminal) = counter();
    let (general_count, on_general) = counter();
    let _subs: Vec<Subscription> = cx.update(|cx| {
        vec![
            TerminalSettings::observe(cx, on_terminal),
            GeneralSettings::observe(cx, on_general),
        ]
    });
    cx.run_until_parked();

    // Unrelated change: only the root-level setting's observer runs.
    cx.update(|cx| update_user_settings::<GeneralSettings>(cx, None, |c| c.ui_scale = Some(2.0)))
        .await
        .unwrap();
    cx.run_until_parked();
    assert_eq!((terminal_count.get(), general_count.get()), (0, 1));

    // The observed setting changes: exactly one notification.
    cx.update(|cx| {
        update_user_settings::<TerminalSettings>(cx, None, |c| c.font_size = Some(20.0))
    })
    .await
    .unwrap();
    cx.run_until_parked();
    assert_eq!((terminal_count.get(), general_count.get()), (1, 1));
    cx.read(|cx| assert_eq!(TerminalSettings::get_global(cx).font_size, 20.0));

    // The edit went to disk with the user's comment intact.
    let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
    assert!(text.starts_with("// mine\n"), "{text}");
    assert!(text.contains("\"font_size\": 20.0, // keep me"), "{text}");
    assert!(text.contains("\"ui_scale\": 2.0"), "{text}");
}

#[gpui::test]
fn reload_path_notifies_on_change_and_keeps_last_good_on_errors(cx: &mut TestAppContext) {
    let _dir = setup(cx, Some(USER));
    let (count, on_change) = counter();
    let _sub = cx.update(|cx| TerminalSettings::observe(cx, on_change));
    cx.run_until_parked();

    // What the hot-reload task does with new file text.
    let reload = |cx: &mut TestAppContext, text: &str| {
        let text = text.to_owned();
        cx.update(|cx| SettingsStore::update_global(cx, |store, _| store.set_user_settings(&text)))
    };

    reload(cx, "{\"terminal\": {\"font_size\": 15}}").unwrap();
    cx.run_until_parked();
    assert_eq!(count.get(), 1);

    // Invalid JSON: error reported, value and observers untouched.
    assert!(reload(cx, "{\"terminal\": ").is_err());
    cx.run_until_parked();
    assert_eq!(count.get(), 1);
    cx.read(|cx| {
        assert_eq!(TerminalSettings::get_global(cx).font_size, 15.0);
        assert!(matches!(
            cx.global::<SettingsStore>().diagnostics(),
            [SettingsDiagnostic::InvalidJson { .. }]
        ));
    });

    // Type error: last good value kept, no notification.
    reload(cx, "{\"terminal\": {\"font_size\": \"huge\"}}").unwrap();
    cx.run_until_parked();
    assert_eq!(count.get(), 1);
    cx.read(|cx| assert_eq!(TerminalSettings::get_global(cx).font_size, 15.0));
}

#[gpui::test]
async fn cluster_overrides_are_read_by_location(cx: &mut TestAppContext) {
    let _dir = setup(cx, Some(USER));
    let prod: ClusterId = "00000000000000ff".parse().unwrap();
    let other: ClusterId = "0000000000000001".parse().unwrap();
    cx.update(|cx| {
        update_user_settings::<TerminalSettings>(cx, Some(prod.clone()), |c| {
            c.font_size = Some(18.0)
        })
    })
    .await
    .unwrap();
    cx.read(|cx| {
        let at = |id| Some(SettingsLocation { cluster: id });
        assert_eq!(TerminalSettings::get(at(&prod), cx).font_size, 18.0);
        assert_eq!(TerminalSettings::get(at(&other), cx).font_size, 13.0);
        assert_eq!(TerminalSettings::get_global(cx).font_size, 13.0);
    });
}

struct FontView {
    renders: usize,
    _settings: Subscription,
}

impl FontView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            renders: 0,
            _settings: TerminalSettings::observe_in(cx, |this, cx| {
                this.renders += 1;
                cx.notify();
            }),
        }
    }
}

#[gpui::test]
fn entity_observers_run_with_the_entity(cx: &mut TestAppContext) {
    let _dir = setup(cx, Some(USER));
    let view = cx.new(FontView::new);
    cx.run_until_parked();
    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, _| {
            store.set_user_settings("{\"ui_scale\": 3}").unwrap();
        })
    });
    cx.run_until_parked();
    // `ui_scale` is unrelated, but dropping the `terminal` override changes the value.
    assert_eq!(view.read_with(cx, |view, _| view.renders), 1);
    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, _| {
            store.set_user_settings("{\"ui_scale\": 4}").unwrap();
        })
    });
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |view, _| view.renders), 1);
}

#[gpui::test]
fn try_get_and_override_global(cx: &mut TestAppContext) {
    cx.read(|cx| assert!(TerminalSettings::try_get(cx).is_none()));
    let _dir = setup(cx, None);
    cx.update(|cx| {
        TerminalSettings::override_global(TerminalSettings { font_size: 99.0 }, cx);
        assert_eq!(TerminalSettings::get_global(cx).font_size, 99.0);
    });
}
