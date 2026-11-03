//! Tests of the settings page: the model built from the schema, the search, the badge's words,
//! and the page in a workspace window over a settings store installed from a temporary
//! directory (`init_with_dir`, no watcher).

mod model;
mod search;
mod view;

use gpui::{AppContext as _, Entity, Modifiers, Point, TestAppContext, VisualTestContext};
use oxikube_workspace::{OpenOptions, test_support::open_workspace};
use tempfile::TempDir;

use super::SettingsPage;
use super::test_support::{fixture_model, install_settings};

/// The workspace window with the page open in it, over the fixture schema.
pub(super) struct Harness {
    pub vcx: VisualTestContext,
    pub page: Entity<SettingsPage>,
    /// The config directory the settings store reads; removed when the test ends.
    _config: TempDir,
}

impl Harness {
    /// Opens the page with `user_settings` as the user's `settings.json`.
    pub fn open(cx: &mut TestAppContext, user_settings: &str) -> Self {
        let (workspace, mut vcx) = open_workspace(cx);
        let config = tempfile::tempdir().expect("a temporary config directory");
        let mut page = None;
        vcx.update(|window, cx| {
            install_settings(config.path(), user_settings, cx);
            let entity = cx.new(|cx| SettingsPage::with_model(fixture_model(), window, cx));
            page = Some(entity.clone());
            workspace.update(cx, |ws, cx| {
                ws.open_item_with(Box::new(entity), OpenOptions::default(), window, cx);
            });
        });
        vcx.run_until_parked();
        Self {
            vcx,
            page: page.expect("the page was built"),
            _config: config,
        }
    }

    pub fn read<R>(&mut self, f: impl FnOnce(&SettingsPage) -> R) -> R {
        let page = self.page.clone();
        self.vcx.read(|cx| f(page.read(cx)))
    }

    pub fn bounds(&mut self, selector: &str) -> Option<gpui::Bounds<gpui::Pixels>> {
        // GPUI wants a `'static` selector: leak the few a test asks for.
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        self.vcx.debug_bounds(selector)
    }

    pub fn is_laid_out(&mut self, selector: &str) -> bool {
        self.bounds(selector).is_some()
    }

    fn centre(&mut self, selector: &str) -> Point<gpui::Pixels> {
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
}
