//! The `editor` settings applied to open manifest editors (E10-S12): hot reload of tab size,
//! wrap, font and validation without touching the text or the undo history, the UI zoom, a
//! cluster's own override, the notice for a rejected value, and the observer going away with
//! the tab.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, AppContext as _, Entity, TestAppContext, UpdateGlobal as _, VisualTestContext, WeakEntity,
};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::schema::JsonSchema;
use oxikube_editor::settings::EditorSettings;
use oxikube_editor::view::{
    EditorHost, EditorViewSink, EditorViews, EditorViewsDeps, ManifestEditor, ManifestEditorParts,
    SchemaSource, VALIDATION_DEBOUNCE,
};
use oxikube_ports::SchemaPort;
use oxikube_settings::SettingsStore;
use oxikube_testkit::fakes::FakeSchemaPort;
use oxikube_ui::editor::EditorApi as _;
use oxikube_ui::{UiScale, set_ui_scale};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, Workspace};
use serde_json::json;

fn cluster() -> ClusterId {
    ClusterId::new("/kubeconfig", &ContextName::new("kind-oxikube"))
}

struct TestSchemas {
    port: Arc<FakeSchemaPort>,
}

impl SchemaSource for TestSchemas {
    fn cluster(&self) -> &ClusterId {
        static CLUSTER: std::sync::OnceLock<ClusterId> = std::sync::OnceLock::new();
        CLUSTER.get_or_init(cluster)
    }
    fn label(&self, _: &App) -> gpui::SharedString {
        "kind-oxikube".into()
    }
    fn port(&self, _: &App) -> Option<Arc<dyn SchemaPort>> {
        Some(self.port.clone())
    }
}

struct NoBus;

impl CommandDispatcher for NoBus {
    fn dispatch(&self, _: oxikube_domain::command::Command, _: &mut App) {}
}

struct Fixture {
    workspace: Entity<Workspace>,
    vcx: VisualTestContext,
    port: Arc<FakeSchemaPort>,
}

/// A workspace, the shipped defaults as the settings store (no file, no watcher thread) and a
/// ConfigMap schema behind a fake `SchemaPort`.
fn setup(cx: &mut TestAppContext) -> Fixture {
    let (workspace, mut vcx) = open_workspace(cx);
    let port = Arc::new(FakeSchemaPort::new());
    port.insert(
        cluster(),
        Gvk::new("", "v1", "ConfigMap"),
        Arc::new(JsonSchema::from_value(&json!({
            "type": "object",
            "properties": {
                "apiVersion": {"type": "string"},
                "kind": {"type": "string"},
                "metadata": {"type": "object"},
                "data": {"type": "object", "additionalProperties": {"type": "string"}}
            }
        }))),
    );
    vcx.update(|_, cx| {
        oxikube_runtime::init_deterministic(cx);
        oxikube_editor::init(cx);
        oxikube_keymap::init_with_text("", oxikube_keymap::KeymapOptions::default(), cx);
        let store = SettingsStore::new(oxikube_assets::default_settings()).expect("defaults");
        cx.set_global(store);
    });
    Fixture {
        workspace,
        vcx,
        port,
    }
}

impl Fixture {
    /// Replaces the user's `settings.json` text, as a reload from the watcher does.
    fn settings(&mut self, text: &str) {
        let text = text.to_owned();
        self.vcx.update(|_, cx| {
            SettingsStore::update_global(cx, |store, _| {
                store.set_user_settings(&text).expect("valid JSON");
            });
        });
        self.vcx.run_until_parked();
    }

    /// Opens an editor holding `text`; with a cluster its schemas are checked.
    fn open(&mut self, text: &str, with_cluster: bool) -> Entity<ManifestEditor> {
        let workspace = self.workspace.clone();
        let schemas: Option<Rc<dyn SchemaSource>> = with_cluster.then(|| {
            Rc::new(TestSchemas {
                port: self.port.clone(),
            }) as Rc<dyn SchemaSource>
        });
        let text = text.to_owned();
        let editor = self.vcx.update(|window, cx| {
            let parts = ManifestEditorParts {
                title: "Untitled-1".into(),
                text,
                schemas,
                dispatcher: Rc::new(NoBus),
                resource: None,
            };
            let editor = cx.new(|cx| ManifestEditor::new(parts, window, cx));
            workspace.update(cx, |ws, cx| ws.open_item(editor.clone(), window, cx));
            editor
        });
        self.vcx.run_until_parked();
        editor
    }

    fn advance(&mut self, by: std::time::Duration) {
        self.vcx.executor().advance_clock(by);
        self.vcx.run_until_parked();
    }

    fn code_state(&mut self, editor: &Entity<ManifestEditor>) -> (usize, bool, u64, String) {
        let code = self.vcx.read(|cx| editor.read(cx).editor().clone());
        self.vcx.update(|window, cx| {
            code.update(cx, |code, cx| {
                let api = code.api(window, cx);
                (
                    api.tab_size(),
                    api.soft_wrap(),
                    api.version(),
                    api.text().to_string(),
                )
            })
        })
    }

    fn squiggles(&mut self, editor: &Entity<ManifestEditor>) -> usize {
        self.vcx.read(|cx| {
            editor
                .read(cx)
                .editor()
                .read(cx)
                .state()
                .read(cx)
                .diagnostics()
                .map_or(0, |set| set.iter().count())
        })
    }
}

fn undo_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-z"
    } else {
        "ctrl-z"
    }
}

#[gpui::test]
fn a_new_editor_starts_with_the_settings_in_effect(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    f.settings(r#"{ "editor": { "tab_size": 6, "soft_wrap": true } }"#);
    let editor = f.open("a: 1\n", false);
    let (tab, wrap, _, _) = f.code_state(&editor);
    assert_eq!((tab, wrap), (6, true));
}

#[gpui::test]
fn soft_wrap_and_tab_size_apply_live_without_losing_text_or_undo(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    let editor = f.open("", false);
    f.vcx.simulate_input("a: 1\n");
    let (tab, wrap, version, text) = f.code_state(&editor);
    assert_eq!((tab, wrap, text.as_str()), (2, false, "a: 1\n"));

    f.settings(r#"{ "editor": { "soft_wrap": true, "tab_size": 4 } }"#);
    let (tab, wrap, after, text) = f.code_state(&editor);
    assert_eq!((tab, wrap), (4, true), "applied at once");
    assert_eq!(after, version, "the buffer was not changed");
    assert_eq!(text, "a: 1\n");

    // The tab key now indents by the new width.
    f.vcx.simulate_keystrokes("tab");
    assert_eq!(f.code_state(&editor).3, "a: 1\n    ");

    // And the undo history is the one from before the change: the indent, then the typing.
    f.vcx.simulate_keystrokes(undo_key());
    assert_eq!(f.code_state(&editor).3, "a: 1\n");
    f.vcx.simulate_keystrokes(undo_key());
    assert_eq!(
        f.code_state(&editor).3,
        "a: 1",
        "undo reaches the newline typed before the change"
    );

    // A change of another setting leaves the wrap alone (and a toggle stays toggled).
    f.settings(r#"{ "editor": { "soft_wrap": true, "tab_size": 4, "mask_secrets": false } }"#);
    assert!(f.code_state(&editor).1);
}

#[gpui::test]
fn another_setting_changing_does_not_undo_a_toggled_wrap(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    let editor = f.open("a: 1\n", false);
    let code = f.vcx.read(|cx| editor.read(cx).editor().clone());
    f.vcx.update(|window, cx| {
        code.update(cx, |code, cx| code.api(window, cx).set_soft_wrap(true));
    });
    f.settings(r#"{ "editor": { "tab_size": 3 } }"#);
    let (tab, wrap, _, _) = f.code_state(&editor);
    assert_eq!(tab, 3);
    assert!(wrap, "the toolbar's wrap survives a change of tab_size");
}

#[gpui::test]
fn the_font_follows_the_setting_and_the_ui_zoom(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    let editor = f.open("a: 1\nb: 2\n", false);
    let code = f.vcx.read(|cx| editor.read(cx).editor().clone());
    let size = |f: &mut Fixture| f.vcx.read(|cx| f32::from(code.read(cx).text_size(cx)));
    let theme_size = size(&mut f);
    assert!(theme_size > 0.0);

    f.settings(r#"{ "editor": { "font_family": "Menlo", "font_size": 14 } }"#);
    assert_eq!(size(&mut f), 14.0);
    let font = f.vcx.read(|cx| code.read(cx).font().clone());
    assert_eq!(font.family.as_deref(), Some("Menlo"));
    assert_eq!(font.size, Some(14.0));

    // 14 points at a zoom of 1.5 is 21 pixels, and the rows follow.
    let row = |f: &mut Fixture| -> f32 {
        f.vcx.run_until_parked();
        f.vcx.update(|window, cx| {
            window.refresh();
            code.update(cx, |code, cx| {
                let api = code.api(window, cx);
                let first = api.range_to_bounds(0..0).map(|b| b.origin.y);
                let second = api.range_to_bounds(5..5).map(|b| b.origin.y);
                match (first, second) {
                    (Some(a), Some(b)) => f32::from(b - a),
                    _ => 0.0,
                }
            })
        })
    };
    let row_at_one = row(&mut f);
    f.vcx.update(|_, cx| set_ui_scale(cx, UiScale::new(1.5)));
    assert_eq!(size(&mut f), 21.0);
    let row_at_one_and_a_half = row(&mut f);
    assert!(row_at_one > 0.0, "the rows were laid out");
    let ratio = row_at_one_and_a_half / row_at_one;
    assert!((ratio - 1.5).abs() < 0.05, "rows follow the zoom: {ratio}");

    // Back to the theme's font.
    f.vcx.update(|_, cx| set_ui_scale(cx, UiScale::IDENTITY));
    f.settings(r#"{ "editor": { "font_family": null, "font_size": null } }"#);
    assert_eq!(size(&mut f), theme_size);
}

const BROKEN: &str = "apiVersion: v1\nkind: ConfigMap\ndata:\n  a: 1\n";

#[gpui::test]
fn validation_can_be_turned_off_and_on_live(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    let editor = f.open(BROKEN, true);
    f.advance(VALIDATION_DEBOUNCE);
    assert_eq!(f.squiggles(&editor), 1, "validated on open");
    let calls = f.port.recorded_calls().len();

    f.settings(r#"{ "editor": { "validation": false } }"#);
    assert_eq!(f.squiggles(&editor), 0, "off clears the squiggles at once");
    let problems = f.vcx.read(|cx| editor.read(cx).model().problems());
    assert_eq!((problems.errors, problems.warnings), (0, 0));

    let (_, _, version_before, _) = f.code_state(&editor);
    f.vcx.simulate_input("# note\n");
    f.advance(VALIDATION_DEBOUNCE * 2);
    let (_, _, version_after, typed) = f.code_state(&editor);
    assert_ne!(
        version_after, version_before,
        "the typing reached the editor"
    );
    assert!(typed.contains("# note"), "{typed}");
    assert_eq!(
        f.squiggles(&editor),
        0,
        "nothing is checked while it is off"
    );
    assert_eq!(
        f.port.recorded_calls().len(),
        calls,
        "no schema is asked for (the cached ConfigMap schema would not be asked for anyway)"
    );

    f.settings(r#"{ "editor": { "validation": true } }"#);
    f.advance(VALIDATION_DEBOUNCE);
    assert_eq!(f.squiggles(&editor), 1, "on checks the open editor again");
}

#[gpui::test]
fn a_cluster_override_reaches_only_that_clusters_editors(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    let with_cluster = f.open("a: 1\n", true);
    let without = f.open("b: 2\n", false);
    let user = format!(
        r#"{{ "editor": {{ "tab_size": 3 }},
              "clusters": {{ "{}": {{ "editor": {{ "tab_size": 8 }} }} }} }}"#,
        cluster().as_str()
    );
    f.settings(&user);
    assert_eq!(f.code_state(&with_cluster).0, 8);
    assert_eq!(f.code_state(&without).0, 3);
    let (a, b) = f.vcx.read(|cx| {
        (
            with_cluster.read(cx).settings(cx).tab_size,
            without.read(cx).settings(cx).tab_size,
        )
    });
    assert_eq!((a, b), (8, 3));
}

#[gpui::test]
fn closing_the_tab_drops_its_settings_observer(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    let editor = f.open("a: 1\n", false);
    let weak: WeakEntity<ManifestEditor> = editor.downgrade();
    let id = editor.entity_id();
    drop(editor);
    let workspace = f.workspace.clone();
    f.vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| ws.close_item(id, window, cx));
    });
    f.vcx.run_until_parked();
    assert!(weak.upgrade().is_none(), "the closed editor is released");
    // The store keeps changing with nobody observing for it.
    f.settings(r#"{ "editor": { "tab_size": 5 } }"#);
    f.settings(r#"{ "editor": { "tab_size": 6 } }"#);
    assert!(weak.upgrade().is_none());
}

struct OneWorkspace {
    workspace: gpui::WeakEntity<Workspace>,
}

impl EditorHost for OneWorkspace {
    fn active_cluster(&self, _: &App) -> Option<ClusterId> {
        None
    }
    fn workspace(&self, _: &ClusterId, _: &App) -> Option<Entity<Workspace>> {
        self.workspace.upgrade()
    }
    fn show(&self, _: &ClusterId, _: &mut gpui::Window, _: &mut App) {}
    fn schemas(&self, _: &ClusterId) -> Rc<dyn SchemaSource> {
        unreachable!("no cluster is shown")
    }
}

#[gpui::test]
fn a_rejected_value_is_told_in_one_notice_that_names_the_default(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    f.settings(r#"{ "editor": { "tab_size": -2, "field_manager": "has space" } }"#);
    let (sink, requests) = EditorViewSink::channel();
    let _ = sink;
    let workspace = f.workspace.clone();
    let views = f.vcx.update(|window, cx| {
        let deps = EditorViewsDeps {
            host: Rc::new(OneWorkspace {
                workspace: workspace.downgrade(),
            }),
            window_workspace: workspace.downgrade(),
            dispatcher: Rc::new(NoBus),
        };
        EditorViews::start(deps, requests, window, cx)
    });
    f.vcx.run_until_parked();
    let toasts = |f: &mut Fixture| {
        let layer = f.vcx.read(|cx| f.workspace.read(cx).toast_layer().clone());
        f.vcx.read(|cx| layer.read(cx).visible())
    };
    let shown = toasts(&mut f);
    assert_eq!(shown.len(), 1, "{shown:?}");
    let message = shown[0].message.to_string();
    assert!(message.contains("editor.tab_size"), "{message}");
    assert!(message.contains("editor.field_manager"), "{message}");
    assert!(message.contains("oxikube"), "names the default: {message}");
    assert!(!message.contains("has space"), "never echoes the value");

    // The defaults were used, and an unrelated settings change does not repeat the notice. The
    // toast carries a key (a repeated push would replace it, not add a second), so the first one
    // is dismissed: a repeat would then bring it back.
    let settings = f.vcx.read(|cx| EditorSettings::resolve(None, cx).clone());
    assert_eq!(
        (settings.tab_size, settings.field_manager.as_str()),
        (2, "oxikube")
    );
    let dismiss = |f: &mut Fixture| {
        let layer = f.vcx.read(|cx| f.workspace.read(cx).toast_layer().clone());
        f.vcx
            .update(|_, cx| layer.update(cx, |layer, cx| layer.dismiss_key("editor-settings", cx)))
    };
    assert!(dismiss(&mut f), "the notice was on screen");
    f.settings(
        r#"{ "editor": { "tab_size": -2, "field_manager": "has space" }, "ui_scale": 1.0 }"#,
    );
    f.vcx.run_until_parked();
    assert!(
        toasts(&mut f).is_empty(),
        "an unrelated change does not tell it again: {:?}",
        toasts(&mut f)
    );

    // Fixing the file clears the problems; nothing is shown.
    f.settings(r#"{ "editor": { "tab_size": 4 } }"#);
    f.vcx.run_until_parked();
    assert_eq!(
        f.vcx.read(|cx| EditorSettings::resolve(None, cx).tab_size),
        4
    );
    assert!(toasts(&mut f).is_empty(), "no problems, no notice");

    // The same mistake made again is news again.
    f.settings(r#"{ "editor": { "tab_size": -2 } }"#);
    f.vcx.run_until_parked();
    let again = toasts(&mut f);
    assert_eq!(again.len(), 1, "{again:?}");
    assert!(again[0].message.to_string().contains("editor.tab_size"));
    drop(views);
}
