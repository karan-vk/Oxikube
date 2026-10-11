//! A kind's own spec reaches the views: selecting its sidebar entry opens the table with the
//! spec's columns, and its detail renderer draws a panel above the Overview.

use futures::executor::block_on;
use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Styled as _, TestAppContext, WeakEntity, Window, div,
};
use oxikube_app::ColumnId;
use oxikube_domain::access::AccessRules;
use oxikube_domain::ids::Gvk;
use oxikube_testkit::discovery_sets;

use super::{PodColumns, pods_with_own_columns, sidebar_panel};
use crate::detail::DetailView;
use crate::detail::tests::fixture::{Detail, pod_ref, web_pod};
use crate::kinds::{DetailRenderer, KindSpec, KindSpecs};
use crate::table::ResourceTable;
use crate::table::tests::fixture::{Fixture, cluster};
use crate::table::tests::p;

#[gpui::test]
fn selecting_an_entry_opens_the_table_with_the_specs_columns(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.vcx.update(|_, cx| {
        KindSpecs::register(cx, KindSpec::new(pods_with_own_columns())).expect("registers");
    });
    let ports = f.ports();
    ports.access.set_rules(AccessRules::all_access());
    ports.discovery.set_kinds(discovery_sets::core());
    ports.resources.insert(p("x", "web-0", "1"));
    block_on(f.sessions.connect(&cluster())).expect("connect");
    f.settle();

    let panel = sidebar_panel(&mut f);
    f.vcx
        .update(|_, cx| panel.update(cx, |panel, cx| panel.activate("workloads/pods", cx)));
    f.settle();

    let tabs = f.tabs.clone();
    let tables: Vec<Entity<ResourceTable>> = f.vcx.update(|_, cx| {
        let tab = tabs.read(cx).tab(&cluster()).cloned().expect("a tab");
        tab.read(cx)
            .workspace()
            .read(cx)
            .items_of_type::<ResourceTable>()
    });
    assert_eq!(tables.len(), 1, "the entry opened the pods table");
    let columns = f.vcx.update(|_, cx| {
        tables[0]
            .read(cx)
            .read_rows(cx, |d| d.layout().visible_ids())
    });
    assert_eq!(
        columns,
        [
            ColumnId::new(ColumnId::NAME),
            ColumnId::new(PodColumns::PHASE)
        ]
    );
    assert_eq!(f.names(&tables[0]), ["web-0"]);
}

#[gpui::test]
fn a_kind_without_its_own_columns_keeps_the_shared_catalogue(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    f.connect_with([p("x", "web-0", "1")]);
    let table = f.open_pods();
    let columns = f
        .vcx
        .update(|_, cx| table.read(cx).read_rows(cx, |d| d.layout().visible_ids()));
    assert!(columns.contains(&ColumnId::new("restarts")), "{columns:?}");
}

/// A kind's panel: the name of the object the detail shows, followed through the detail.
struct NamePanel {
    detail: WeakEntity<DetailView>,
}

impl Render for NamePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self
            .detail
            .upgrade()
            .and_then(|d| d.read(cx).resource().map(|r| r.meta.name.to_string()))
            .unwrap_or_default();
        div()
            .debug_selector(|| "pod-panel".to_owned())
            .w_full()
            .child(format!("pod {name}"))
    }
}

#[gpui::test]
fn a_kinds_detail_renderer_draws_its_panel_above_the_overview(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    d.f.vcx.update(|_, cx| {
        let renderer = DetailRenderer::panel(|detail, _, cx| {
            cx.new(|cx| {
                if let Some(view) = detail.upgrade() {
                    cx.observe(&view, |_, _, cx| cx.notify()).detach();
                }
                NamePanel { detail }
            })
            .into()
        });
        let spec = KindSpec::new(pods_with_own_columns()).with_detail(renderer);
        KindSpecs::register(cx, spec).expect("registers");
    });
    let view = d.open(&pod_ref("web-0"));
    assert!(d.shown("detail-kind-panel"));
    assert!(d.shown("pod-panel"));
    assert!(d.shown("detail-overview"), "the generic rows stay");
    assert!(d.read(&view, |v| v.kind_panel().is_some()));
    let name = d.read(&view, |v| v.resource().map(|r| r.meta.name.to_string()));
    assert_eq!(name.as_deref(), Some("web-0"));

    // Another kind has no panel.
    let generic = d.f.vcx.update(|_, cx| {
        KindSpecs::detail_for(cx, &Gvk::new("apps", "v1", "ReplicaSet")).is_generic()
    });
    assert!(generic);
}
