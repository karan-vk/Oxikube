//! Every kind reachable from its sidebar entry: the table lists the seeded object with the
//! documented columns, and the object's detail opens in the drawer with the kind's panel.

use futures::executor::block_on;
use gpui::{Entity, TestAppContext};
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_resources_ui::detail::{DetailDrawer, DetailView};
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::TestPorts;
use oxikube_testkit::sections::{KindCase, catalog};
use oxikube_workspace::sidebar::{DEFINITIONS_ENTRY, SidebarPanel};

use super::super::App;
use super::{SEED, target};

impl App {
    /// Activates `case`'s sidebar entry and returns the table it opened.
    fn open_from_sidebar(&mut self, case: &KindCase) -> Entity<ResourceTable> {
        let ws = self.tab_workspace();
        let panel = self
            .vcx
            .update(|_, cx| ws.read(cx).panel::<SidebarPanel>())
            .expect("the tab has its sidebar");
        let entry = if case.section == "custom-resources" {
            DEFINITIONS_ENTRY.to_owned()
        } else {
            case.entry()
        };
        let listed = self
            .vcx
            .update(|_, cx| panel.read(cx).row(&entry).is_some());
        assert!(listed, "the sidebar lists {entry}");
        self.vcx
            .update(|_, cx| panel.update(cx, |panel, cx| panel.activate(&entry, cx)));
        self.tick();
        let gvk = case.gvk();
        self.vcx
            .update(|_, cx| {
                ws.read(cx)
                    .items_of_type::<ResourceTable>()
                    .into_iter()
                    .find(|table| table.read(cx).gvk() == &gvk)
            })
            .unwrap_or_else(|| panic!("{entry} opened no {} table", case.kind))
    }

    /// The detail view the cluster tab's drawer shows.
    fn drawer_view(&mut self) -> Option<Entity<DetailView>> {
        let ws = self.tab_workspace();
        self.vcx.update(|_, cx| {
            ws.read(cx)
                .panel::<DetailDrawer>()
                .and_then(|drawer| drawer.read(cx).view().cloned())
        })
    }
}

#[gpui::test]
fn every_kind_opens_from_its_entry_with_its_columns_and_its_detail(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.connect_sections();
    for case in catalog() {
        let table = app.open_from_sidebar(case);
        let name = case.seed_name(&SEED);
        let (rows, columns) = app.vcx.update(|_, cx| {
            table.read(cx).read_rows(cx, |d| {
                let rows: Vec<String> = d.rows().iter().map(|o| o.name().to_owned()).collect();
                let columns: Vec<String> = d
                    .layout()
                    .columns()
                    .map(|(c, _)| format!("{}{}", c.title, if c.wide { "*" } else { "" }))
                    .collect();
                (rows, columns)
            })
        });
        assert!(
            rows.contains(&name),
            "{} lists the seeded {name}: {rows:?}",
            case.kind
        );
        assert_eq!(columns, case.columns, "{}'s table columns", case.kind);
        // The rows are drawn: every cell of the seed computed without a panic.
        assert!(app.drawn("td-0-0"), "{}'s first row is drawn", case.kind);

        // The detail: `resource::Open` (Enter on the row, or "Show Details" in its menu) opens
        // the drawer on the object, which draws with the kind's own panel.
        let target = target(case);
        let opened = block_on(app.bus().dispatch(
            Command::ResourceOpen {
                target: target.clone(),
            },
            DispatchContext::new(Initiator::Ui, "alice"),
        ));
        assert!(opened.is_ok(), "{}: {opened:?}", case.kind);
        app.tick();
        let view = app.drawer_view().expect("the drawer shows a detail");
        assert!(app.drawn("detail-view"), "{}'s detail is drawn", case.kind);
        let (shown, panel) = app.vcx.update(|_, cx| {
            let view = view.read(cx);
            (view.target().clone(), view.kind_panel().is_some())
        });
        assert_eq!(shown, target, "{}", case.kind);
        assert_eq!(panel, case.panel, "{}'s panel", case.kind);
    }
}
