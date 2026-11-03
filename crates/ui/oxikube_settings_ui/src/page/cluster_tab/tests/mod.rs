//! Tests of the per-cluster tab: the model from the schema, the precedence data, the cluster
//! list, and the page in a workspace window over a settings store installed from a temporary
//! directory (`init_with_dir`, no watcher).

mod model;
mod precedence;
mod view;

use std::path::PathBuf;
use std::time::Duration;

use gpui::{AppContext as _, Entity, Modifiers, TestAppContext, VisualTestContext};
use oxikube_app::CatalogEntry;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_workspace::{OpenOptions, test_support::open_workspace};
use serde_json::Value;
use tempfile::TempDir;

use crate::page::test_support::{fixture_model, install_settings};
use crate::page::{Scope, SettingsPage};

/// The id of the cluster `name`.
pub(super) fn id(name: &str) -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new(name))
}

/// The catalog entry of the context `name`.
pub(super) fn entry(name: &str) -> CatalogEntry {
    CatalogEntry {
        context: ClusterContext::new(
            id(name),
            ContextName::new(name),
            SourceId("kubeconfig:~/.kube/config".into()),
        ),
        source: None,
        favourite: false,
        last_used: None,
    }
}

/// The page, open on the per-cluster tab of the first of `contexts`.
pub(super) struct Harness {
    pub vcx: VisualTestContext,
    pub page: Entity<SettingsPage>,
    config: TempDir,
}

impl Harness {
    /// The page over `user_settings`, switched to the per-cluster tab, with the catalog listing
    /// `contexts` (the first is picked).
    pub fn open(cx: &mut TestAppContext, user_settings: &str, contexts: &[&str]) -> Self {
        let (workspace, mut vcx) = open_workspace(cx);
        let config = tempfile::tempdir().expect("a temporary config directory");
        let mut page = None;
        vcx.update(|window, cx| {
            install_settings(config.path(), user_settings, cx);
            let entity = cx.new(|cx| SettingsPage::with_model(fixture_model(), window, cx));
            entity.update(cx, |page, cx| {
                page.set_catalog_entries(contexts.iter().map(|name| entry(name)).collect(), cx);
                page.set_scope(Scope::Cluster, window, cx);
            });
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

    pub fn click(&mut self, selector: &str) {
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        let at = self
            .vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} was not laid out"))
            .center();
        self.vcx.simulate_click(at, Modifiers::none());
        self.vcx.run_until_parked();
    }

    /// Scrolls the content so the row of the field at `path` is in view (the rows of this tab
    /// are tall; the test window is not).
    pub fn reveal(&mut self, path: &str) {
        self.page.update(&mut self.vcx, |page, cx| {
            let (section, field) = page.model.find_field(path).expect("a field of the tab");
            assert_eq!(section, page.selected, "{path} is in the shown section");
            page.content_scroll
                .scroll_to_item(field + 1 + page.header_offset());
            cx.notify();
        });
        self.vcx.run_until_parked();
    }

    /// Scrolls back to the cluster list.
    pub fn scroll_to_top(&mut self) {
        self.page.update(&mut self.vcx, |page, cx| {
            page.content_scroll.scroll_to_item(0);
            cx.notify();
        });
        self.vcx.run_until_parked();
    }

    pub fn is_laid_out(&mut self, selector: &str) -> bool {
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        self.vcx.debug_bounds(selector).is_some()
    }

    pub fn type_into(&mut self, path: &str, text: &str) {
        self.reveal(path);
        self.click(&format!("settings-input-{path}"));
        self.vcx.simulate_keystrokes("cmd-a backspace");
        self.vcx.simulate_input(text);
        self.vcx.run_until_parked();
    }

    pub fn pause(&mut self, by: Duration) {
        self.vcx.executor().advance_clock(by);
        self.vcx.run_until_parked();
    }

    pub fn file(&self) -> String {
        std::fs::read_to_string(self.file_path()).expect("settings.json exists")
    }

    pub fn file_path(&self) -> PathBuf {
        self.config.path().join("settings.json")
    }

    /// The user file as JSON (comments and commas stripped by the lenient parser).
    pub fn file_json(&self) -> Value {
        let object =
            oxikube_settings::jsonc::parse_jsonc_object(&self.file()).expect("valid JSONC");
        Value::Object(object)
    }

    pub fn page<R>(&mut self, f: impl FnOnce(&SettingsPage) -> R) -> R {
        let page = self.page.clone();
        self.vcx.read(|cx| f(page.read(cx)))
    }
}
