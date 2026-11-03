//! The one test harness of the settings page (E21-S10): a workspace window with the page open
//! in it, over a settings store installed from a temporary config directory (`init_with_dir`,
//! no watcher). The page, controls and cluster-tab tests all build on it instead of keeping
//! their own copy; each adds its few specific helpers in an `impl Harness` of its own.

use std::time::Duration;

use gpui::{
    AppContext as _, Bounds, Context, Entity, Modifiers, Pixels, Point, TestAppContext,
    VisualTestContext, Window,
};
use oxikube_testkit::config_dir::TempConfigDir;
use oxikube_workspace::{OpenOptions, test_support::open_workspace};
use serde_json::Value;

use super::test_support::{fixture_model, install_settings};
use super::{SettingsPage, SettingsPageModel};

/// The workspace window with the page open in it.
pub(crate) struct Harness {
    pub vcx: VisualTestContext,
    pub page: Entity<SettingsPage>,
    /// The config directory the settings store reads; removed when the test ends.
    config: TempConfigDir,
}

impl Harness {
    /// Opens the fixture-schema page with `user_settings` as the user's `settings.json`.
    pub fn open(cx: &mut TestAppContext, user_settings: &str) -> Self {
        Self::open_with(cx, fixture_model(), user_settings, |_, _, _| {})
    }

    /// Opens a page over `model`, running `setup` on it before it is mounted.
    pub fn open_with(
        cx: &mut TestAppContext,
        model: SettingsPageModel,
        user_settings: &str,
        setup: impl FnOnce(&mut SettingsPage, &mut Window, &mut Context<SettingsPage>),
    ) -> Self {
        let (workspace, mut vcx) = open_workspace(cx);
        let config = TempConfigDir::with_settings(user_settings);
        let mut page = None;
        vcx.update(|window, cx| {
            install_settings(config.path(), user_settings, cx);
            let entity = cx.new(|cx| SettingsPage::with_model(model, window, cx));
            entity.update(cx, |page, cx| setup(page, window, cx));
            page = Some(entity.clone());
            workspace.update(cx, |ws, cx| {
                ws.open_item_with(Box::new(entity), OpenOptions::default(), window, cx);
            });
        });
        vcx.run_until_parked();
        Self {
            vcx,
            page: page.expect("the page was built"),
            config,
        }
    }

    pub fn read<R>(&mut self, f: impl FnOnce(&SettingsPage) -> R) -> R {
        let page = self.page.clone();
        self.vcx.read(|cx| f(page.read(cx)))
    }

    pub fn bounds(&mut self, selector: &str) -> Option<Bounds<Pixels>> {
        // GPUI wants a `'static` selector: leak the few a test asks for.
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        self.vcx.debug_bounds(selector)
    }

    pub fn is_laid_out(&mut self, selector: &str) -> bool {
        self.bounds(selector).is_some()
    }

    fn centre(&mut self, selector: &str) -> Point<Pixels> {
        self.bounds(selector)
            .unwrap_or_else(|| panic!("{selector} was not laid out"))
            .center()
    }

    pub fn click(&mut self, selector: &str) {
        let at = self.centre(selector);
        self.vcx.simulate_click(at, Modifiers::none());
        self.vcx.run_until_parked();
    }

    /// Types into the focused element (the search field, once the page has the focus).
    pub fn type_text(&mut self, text: &str) {
        self.vcx.simulate_input(text);
        self.vcx.run_until_parked();
    }

    pub fn keys(&mut self, keystrokes: &str) {
        self.vcx.simulate_keystrokes(keystrokes);
        self.vcx.run_until_parked();
    }

    /// Types `text` into the input laid out at `selector`, replacing its content, as a person
    /// would: click to focus, select all, type.
    pub fn type_at(&mut self, selector: &str, text: &str) {
        self.click(selector);
        self.keys("secondary-a backspace");
        if !text.is_empty() {
            self.type_text(text);
        }
    }

    /// Types `text` into the input of the field at `path`.
    pub fn type_into(&mut self, path: &str, text: &str) {
        self.type_at(&format!("settings-input-{path}"), text);
    }

    /// Lets `by` of the (deterministic) clock pass: the write debounce, without a sleep.
    pub fn pause(&mut self, by: Duration) {
        self.vcx.executor().advance_clock(by);
        self.vcx.run_until_parked();
    }

    /// The text of the user's `settings.json`.
    pub fn file(&self) -> String {
        self.config.settings_text()
    }

    /// The user file as JSON (comments and commas stripped by the lenient parser).
    pub fn file_json(&self) -> Value {
        let object =
            oxikube_settings::jsonc::parse_jsonc_object(&self.file()).expect("valid JSONC");
        Value::Object(object)
    }
}
