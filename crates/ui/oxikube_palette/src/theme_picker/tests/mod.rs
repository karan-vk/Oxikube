//! Tests of the theme picker (E22-S08).
//!
//! | File | Covers |
//! |---|---|
//! | `picker.rs` | the list (registry, groups, current marked, slot caption), arrows preview, Escape and a click outside revert, Enter commits, filtering |
//! | `commit.rs` | what Enter writes to `settings.json` (static and system forms, comments kept) |

mod commit;
mod picker;

use std::rc::Rc;

use gpui::{Entity, TestAppContext, VisualTestContext};
use oxikube_keymap::{KeymapOptions, KeymapPlatform};
use oxikube_theme::{ActiveTheme, Appearance, SystemAppearance};
use oxikube_workspace::Workspace;
use oxikube_workspace::test_support::open_workspace;

use super::{ThemeDelegate, ThemeHost};
use crate::picker::Picker;

/// Ayu (dark, light, mirage), next to the bundled One Dark and One Light.
const AYU: &str = include_str!("../../../../../platform/oxikube_theme/tests/fixtures/ayu.json");

/// A user `settings.json` with a comment, in the system-following form.
pub const SYSTEM_SETTINGS: &str = "{\n  // keep me\n  \"theme\": { \"mode\": \"system\", \"light\": \"One Light\", \"dark\": \"One Dark\" },\n  \"zoom\": 1\n}\n";

/// A workspace window with the shipped keymap, the theme registry (One and Ayu) over a temp
/// config dir, and the picker's host.
pub struct Fixture {
    pub workspace: Entity<Workspace>,
    pub vcx: VisualTestContext,
    pub host: Rc<ThemeHost>,
    pub config: tempfile::TempDir,
}

impl Fixture {
    pub fn new(cx: &mut TestAppContext, system: Appearance, settings: &str) -> Self {
        let (workspace, mut vcx) = open_workspace(cx);
        let config = tempfile::tempdir().expect("create temp config dir");
        std::fs::write(config.path().join("settings.json"), settings).expect("write settings");
        let themes = oxikube_theme::user_dir::themes_dir(config.path());
        std::fs::create_dir_all(&themes).expect("create themes dir");
        std::fs::write(themes.join("ayu.json"), AYU).expect("write ayu.json");
        vcx.update(|_, cx| {
            let options = KeymapOptions {
                platform: if cfg!(target_os = "macos") {
                    KeymapPlatform::MacOs
                } else {
                    KeymapPlatform::Linux
                },
                ..KeymapOptions::default()
            };
            oxikube_keymap::init_with_text("", options, cx);
            oxikube_settings::init_with_dir(config.path(), cx);
            SystemAppearance::init(cx);
            SystemAppearance::set(cx, system);
            oxikube_theme::init_with_dir(Some(&themes), cx);
            // The shipped set grows (Oxikube, Ayu, Gruvbox); these tests pin One plus the Ayu
            // fixture so their arrow-key walks do not depend on it.
            gpui::UpdateGlobal::update_global(
                cx,
                |registry: &mut oxikube_theme::registry::ThemeRegistry, _| {
                    registry.remove_by_source(&oxikube_theme::registry::ThemeSource::Bundled);
                    let one = oxikube_assets::BUNDLED_THEME_FAMILIES
                        .iter()
                        .find(|file| file.file == "one.json")
                        .expect("one.json is bundled");
                    let imported =
                        oxikube_theme::import::import_family(one.json).expect("one.json");
                    registry.insert_bundled(imported.family);
                },
            );
        });
        let host = Rc::new(ThemeHost::new(&workspace));
        let mut fixture = Self {
            workspace,
            vcx,
            host,
            config,
        };
        fixture.settle();
        fixture
    }

    pub fn open(&mut self) {
        let host = self.host.clone();
        self.vcx.update(|window, cx| host.toggle(window, cx));
        self.settle();
    }

    pub fn settle(&mut self) {
        self.vcx.run_until_parked();
        self.vcx.update(|window, cx| window.draw(cx).clear(cx));
    }

    pub fn keys(&mut self, keys: &str) {
        self.vcx.simulate_keystrokes(keys);
        self.settle();
    }

    pub fn type_text(&mut self, text: &str) {
        self.vcx.simulate_input(text);
        self.settle();
    }

    pub fn picker(&mut self) -> Option<Entity<Picker<ThemeDelegate>>> {
        let workspace = self.workspace.clone();
        self.vcx.update(|_, cx| {
            workspace
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<Picker<ThemeDelegate>>()
        })
    }

    /// The name of the theme the window shows.
    pub fn active(&mut self) -> String {
        self.vcx.update(|_, cx| ActiveTheme::get(cx).name.clone())
    }

    /// The name of the selected theme row.
    pub fn selected_name(&mut self) -> Option<String> {
        let picker = self.picker().expect("the picker is open");
        self.vcx
            .update(|_, cx| picker.read(cx).delegate.selected_name().map(str::to_owned))
    }

    pub fn settings_text(&self) -> String {
        std::fs::read_to_string(self.config.path().join("settings.json")).expect("read settings")
    }
}
