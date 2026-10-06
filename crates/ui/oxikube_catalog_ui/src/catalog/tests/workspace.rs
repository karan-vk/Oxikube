//! The catalog as a tab of the workspace window.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, TestAppContext};
use oxikube_app::{ClusterCatalog, ClusterSessionManager};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use oxikube_workspace::{Item as _, OpenOptions, test_support::open_workspace};

use super::contexts;
use crate::catalog::test_support::{RecordingDispatcher, source};
use crate::{CatalogDeps, CatalogView};

#[gpui::test]
fn the_catalog_opens_as_a_tab_and_opening_it_twice_shows_the_open_one(cx: &mut TestAppContext) {
    let (workspace, mut vcx) = open_workspace(cx);
    vcx.update(|_, cx| oxikube_runtime::init_deterministic(cx));
    let source = Arc::new(
        FakeClusterSourcePort::new()
            .with_sources([source()])
            .with_contexts(contexts(3)),
    );
    let clock = Arc::new(FakeClockPort::default());
    let deps = CatalogDeps {
        catalog: ClusterCatalog::new(
            source.clone(),
            Arc::new(FakeStatePort::new()),
            clock.clone(),
        ),
        sessions: ClusterSessionManager::new(
            Arc::new(FakeClusterConnectorPort::new()),
            source,
            clock.clone(),
        ),
        dispatcher: Rc::new(RecordingDispatcher::new()),
        clock,
    };
    for _ in 0..2 {
        vcx.update(|window, cx| {
            let view = cx.new(|cx| CatalogView::new(deps.clone(), window, cx));
            workspace.update(cx, |ws, cx| {
                ws.open_item_with(
                    Box::new(view),
                    OpenOptions {
                        reuse_existing: true,
                        ..OpenOptions::default()
                    },
                    window,
                    cx,
                );
            });
        });
        vcx.run_until_parked();
    }
    vcx.update(|_, cx| {
        let views = workspace.read(cx).items_of_type::<CatalogView>();
        assert_eq!(views.len(), 1, "the second open reused the first");
        let tab = views[0].read(cx).tab_content(cx);
        assert_eq!(tab.title.as_ref(), "Clusters");
        assert_eq!(views[0].read(cx).model().total(), 3, "its read finished");
    });
}
