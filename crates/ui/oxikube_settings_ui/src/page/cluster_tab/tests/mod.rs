//! Tests of the per-cluster tab: the model from the schema, the precedence data, the cluster
//! list, and the page in a workspace window over a settings store installed from a temporary
//! directory (`init_with_dir`, no watcher).

mod model;
mod precedence;
mod view;

use crate::page::Scope;
use crate::page::test_support::fixture_model;
use gpui::TestAppContext;
use oxikube_app::CatalogEntry;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, SourceId};

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
pub(super) use crate::page::harness::Harness;

impl Harness {
    /// The page over `user_settings`, switched to the per-cluster tab, with the catalog listing
    /// `contexts` (the first is picked).
    pub fn open_clusters(cx: &mut TestAppContext, user_settings: &str, contexts: &[&str]) -> Self {
        Self::open_with(cx, fixture_model(), user_settings, |page, window, cx| {
            page.set_catalog_entries(contexts.iter().map(|name| entry(name)).collect(), cx);
            page.set_scope(Scope::Cluster, window, cx);
        })
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

    /// Like the shared `type_into`, after scrolling the field into view.
    pub fn type_into_revealed(&mut self, path: &str, text: &str) {
        self.reveal(path);
        self.type_into(path, text);
    }
}
