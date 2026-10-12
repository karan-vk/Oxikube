//! The registry against the matrix: what the sidebar lists, each kind's columns, its own row
//! actions, its panel, and the actions its spec declares.

use std::sync::Arc;

use gpui::TestAppContext;
use oxikube_app::{ColumnProvider, ColumnsContext, CoreColumns};
use oxikube_domain::Capabilities;
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::Gvk;
use oxikube_resources_ui::kinds::KindSpecs;
use oxikube_testkit::TestPorts;
use oxikube_testkit::sections::{KindCase, catalog, column_label};

use super::super::App;
use super::{served, unknown_kind};
use crate::app_state::AppState;

/// Actions a kind's spec may declare without a row action: they live on a key or a button of the
/// table or the panel instead of the row menu.
const NOT_ON_ROWS: &[CommandId] = &[
    // `shift-f` on a pod; the port-forward dialog.
    CommandId::POD_PORT_FORWARD,
    // Per port, in the Service's panel.
    CommandId::SERVICE_OPEN_IN_BROWSER,
    // The Secrets table's "New" button.
    CommandId::SECRET_CREATE,
];

/// `columns`' titles as the matrix writes them (a wide one ends in `*`).
fn titles(provider: &dyn ColumnProvider, gvk: &Gvk) -> Vec<String> {
    provider
        .columns(gvk, Capabilities::all())
        .iter()
        .map(|c| column_label(&c.title, c.wide))
        .collect()
}

#[gpui::test]
fn the_sidebar_lists_exactly_the_matrixs_kinds_in_its_order(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let tree = app
        .vcx
        .update(|_, cx| KindSpecs::registry(cx).sidebar_tree(None));
    let listed: Vec<String> = tree
        .sections
        .iter()
        .flat_map(|s| {
            s.entries
                .iter()
                .map(move |e| format!("{}/{}", s.section.id, e.meta.plural))
        })
        .collect();
    let documented: Vec<String> = catalog()
        .filter(|c| c.section != "custom-resources")
        .map(KindCase::entry)
        .collect();
    assert_eq!(
        listed, documented,
        "a kind was added, moved or dropped: update oxikube_testkit::sections with it"
    );
}

#[gpui::test]
fn every_kind_has_its_documented_columns_panel_and_scope(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let sessions = app
        .vcx
        .update(|_, cx| AppState::global(cx).services().sessions.clone());
    let context = ColumnsContext {
        cluster: TestPorts::cluster_id(),
        store: None,
        sessions,
        shared: Arc::new(CoreColumns::new()),
    };
    for case in catalog() {
        let (meta, panel) = app.vcx.update(|_, cx| {
            let gvk = case.gvk();
            (
                KindSpecs::kind_for(cx, &gvk),
                !KindSpecs::detail_for(cx, &gvk).is_generic(),
            )
        });
        assert!(!meta.is_generic(), "{} is registered", case.kind);
        assert_eq!(meta.scope, case.scope, "{}", case.kind);
        assert_eq!(meta.plural.as_ref(), case.plural, "{}", case.kind);
        assert_eq!(panel, case.panel, "{}: a panel of its own", case.kind);
        // The kind's own provider (static or per cluster), else the shared core catalogue.
        let provider = meta
            .columns
            .provider()
            .cloned()
            .or_else(|| meta.columns.build(&context))
            .unwrap_or_else(|| context.shared.clone());
        assert_eq!(
            titles(provider.as_ref(), &case.gvk()),
            case.columns,
            "{}'s columns",
            case.kind
        );
    }
}

#[gpui::test]
fn every_kinds_rows_offer_its_documented_actions_and_its_spec_declares_them(
    cx: &mut TestAppContext,
) {
    let mut app = App::start(cx, TestPorts::seeded());
    let rows = app.row_actions();
    let generic: Vec<CommandId> = rows
        .actions_for(&unknown_kind(), Capabilities::all())
        .iter()
        .map(|a| a.meta().id)
        .collect();
    assert!(generic.contains(&CommandId::RESOURCE_DELETE), "{generic:?}");
    for case in catalog() {
        let offered: Vec<_> = rows
            .actions_for(&served(case), Capabilities::all())
            .into_iter()
            .filter(|a| !generic.contains(&a.meta().id))
            .collect();
        let ids: Vec<CommandId> = offered.iter().map(|a| a.meta().id).collect();
        assert_eq!(ids, case.actions, "{}'s own row actions", case.kind);

        // The spec's declared actions (what the palette, the MCP server and the Secret table's
        // button read) agree with the rows: each is offered on a row or is a key or button, and
        // every mutation a row offers is declared.
        let meta = app.vcx.update(|_, cx| KindSpecs::kind_for(cx, &case.gvk()));
        for declared in &meta.actions {
            assert!(
                ids.contains(declared) || NOT_ON_ROWS.contains(declared),
                "{} declares {declared}, which no row, key or button offers",
                case.kind
            );
        }
        for action in offered.iter().filter(|a| a.meta().mutating) {
            assert!(
                meta.offers(action.meta().id),
                "{}'s rows offer {} but its spec does not declare it",
                case.kind,
                action.meta().id
            );
        }
    }
}
