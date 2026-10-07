//! Dockable items (E09-S07): opening one in the bottom dock, moving it between the panes and the
//! dock (by API and by dragging its tab) without recreating it, the dock's focus, and saving and
//! restoring the items of a dock.

use gpui::{Entity, Modifiers, MouseButton, px};

use super::*;

/// Opens a dockable [`TestItem`] titled `title` in the bottom dock.
fn open_docked(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, title: &str) -> EntityId {
    let title = title.to_owned();
    let id = vcx.update(|window, cx| {
        let item = cx.new(|cx| TestItem::new(title, cx).dockable());
        ws.update(cx, |ws, cx| {
            ws.open_item_in_dock(Box::new(item), DockPosition::Bottom, true, window, cx)
        })
    });
    vcx.run_until_parked();
    id.expect("the item opened in the dock")
}

/// The test item `id`.
fn test_item(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    id: EntityId,
) -> Entity<TestItem> {
    vcx.update(|_, cx| {
        ws.read(cx)
            .item(id)
            .and_then(|item| item.downcast::<TestItem>())
            .expect("an open test item")
    })
}

/// Drags the element tagged `from` onto the element tagged `to` with the mouse.
fn drag(vcx: &mut VisualTestContext, from: &'static str, to: &'static str) {
    let from = center(bounds(vcx, from).unwrap_or_else(|| panic!("{from} is drawn")));
    let to = center(bounds(vcx, to).unwrap_or_else(|| panic!("{to} is drawn")));
    vcx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(
        gpui::point(from.x + px(10.), from.y + px(4.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    vcx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    vcx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
}

fn item_dock(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    id: EntityId,
) -> Option<DockPosition> {
    vcx.update(|_, cx| ws.read(cx).item_dock(id, cx))
}

#[gpui::test]
fn a_dockable_item_opens_displayed_and_focused_in_the_dock(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open(&ws, &mut vcx, "a");
    let panel = add_panel(&ws, &mut vcx, DockPosition::Bottom, "terminals", |_| {});
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.toggle_dock(DockPosition::Bottom, window, cx)
        })
    });
    assert!(!dock(&ws, &mut vcx, DockPosition::Bottom).is_open());

    let term = open_docked(&ws, &mut vcx, "zsh");

    let bottom = dock(&ws, &mut vcx, DockPosition::Bottom);
    assert!(
        bottom.is_open(),
        "opening an item in a closed dock opens it"
    );
    assert_eq!(bottom.items(), [term]);
    assert_eq!(bottom.active_item(), Some(term), "displayed");
    assert_eq!(bottom.panels(), [panel.entity_id()], "the panel stays");
    assert_eq!(item_dock(&ws, &mut vcx, term), Some(DockPosition::Bottom));
    assert!(item_focused(&ws, &mut vcx, term));
    assert!(bounds(&mut vcx, "item-zsh").is_some(), "drawn in the dock");
    assert_eq!(panes(&ws, &mut vcx).len(), 1, "the centre is untouched");
}

#[gpui::test]
fn only_dockable_items_open_in_a_dock_and_only_where_a_dock_exists(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let opened = vcx.update(|window, cx| {
        let item = TestItem::build("plain", cx);
        let dockable = cx.new(|cx| TestItem::new("dockable", cx).dockable());
        ws.update(cx, |ws, cx| {
            let plain =
                ws.open_item_in_dock(Box::new(item), DockPosition::Bottom, true, window, cx);
            // No panel was added at the bottom: there is no dock to open it in.
            let no_dock =
                ws.open_item_in_dock(Box::new(dockable), DockPosition::Bottom, true, window, cx);
            (plain, no_dock)
        })
    });
    assert_eq!(opened, (None, None));
    add_panel(&ws, &mut vcx, DockPosition::Bottom, "terminals", |_| {});
    let refused = vcx.update(|window, cx| {
        let item = TestItem::build("plain", cx);
        ws.update(cx, |ws, cx| {
            ws.open_item_in_dock(Box::new(item), DockPosition::Bottom, true, window, cx)
        })
    });
    assert_eq!(refused, None, "a centre-only item never enters a dock");
    assert!(vcx.update(|_, cx| ws.read(cx).items().next().is_none()));
}

#[gpui::test]
fn moving_between_a_pane_and_the_dock_keeps_the_same_item(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open(&ws, &mut vcx, "a");
    add_panel(&ws, &mut vcx, DockPosition::Bottom, "terminals", |_| {});
    let term = open_with(
        &ws,
        &mut vcx,
        "zsh",
        OpenOptions::default(),
        TestItem::dockable,
    );
    let entity = test_item(&ws, &mut vcx, term);
    assert_eq!(item_dock(&ws, &mut vcx, term), None, "opened in the centre");

    let moved = vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.move_item_to_dock(term, DockPosition::Bottom, window, cx)
        })
    });
    vcx.run_until_parked();
    assert!(moved);
    assert_eq!(item_dock(&ws, &mut vcx, term), Some(DockPosition::Bottom));
    assert!(item_focused(&ws, &mut vcx, term));
    assert!(
        !panes(&ws, &mut vcx)[0].items().contains(&term),
        "it left the pane"
    );

    let pane = panes(&ws, &mut vcx)[0].id();
    vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.move_item(term, pane, None, window, cx)));
    vcx.run_until_parked();
    assert_eq!(item_dock(&ws, &mut vcx, term), None, "back in the pane");
    assert!(panes(&ws, &mut vcx)[0].items().contains(&term));

    // The same entity all along, never closed.
    assert_eq!(test_item(&ws, &mut vcx, term), entity);
    assert_eq!(vcx.update(|_, cx| entity.read(cx).closed.get()), 0);
    assert!(vcx.update(|_, cx| ws.read(cx).closed_items().is_empty()));
}

#[gpui::test]
fn dragging_a_dockable_tab_onto_the_dock_keeps_it_there_and_back(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open(&ws, &mut vcx, "a");
    add_panel(&ws, &mut vcx, DockPosition::Bottom, "terminals", |_| {});
    let term = open_with(
        &ws,
        &mut vcx,
        "zsh",
        OpenOptions::default(),
        TestItem::dockable,
    );
    let entity = test_item(&ws, &mut vcx, term);

    drag(&mut vcx, "tab-zsh", "panel-tab-terminals");

    assert_eq!(
        item_dock(&ws, &mut vcx, term),
        Some(DockPosition::Bottom),
        "a dockable item stays where it was dropped"
    );
    assert_eq!(dock(&ws, &mut vcx, DockPosition::Bottom).items(), [term]);

    drag(&mut vcx, "tab-zsh", "tab-a");

    assert_eq!(
        item_dock(&ws, &mut vcx, term),
        None,
        "dragged back to the pane"
    );
    assert!(panes(&ws, &mut vcx)[0].items().contains(&term));
    assert_eq!(test_item(&ws, &mut vcx, term), entity, "the same item");
    assert_eq!(vcx.update(|_, cx| entity.read(cx).closed.get()), 0);
}

#[gpui::test]
fn toggling_the_dock_focuses_its_item_and_gives_focus_back(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let a = open(&ws, &mut vcx, "a");
    add_panel(&ws, &mut vcx, DockPosition::Bottom, "terminals", |_| {});
    let term = open_docked(&ws, &mut vcx, "zsh");
    assert!(item_focused(&ws, &mut vcx, term));

    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.toggle_dock(DockPosition::Bottom, window, cx)
        })
    });
    vcx.run_until_parked();
    assert!(
        item_focused(&ws, &mut vcx, a),
        "closing the dock hands focus back"
    );

    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.toggle_dock(DockPosition::Bottom, window, cx)
        })
    });
    vcx.run_until_parked();
    assert!(
        item_focused(&ws, &mut vcx, term),
        "opening it focuses the displayed item"
    );
}

#[gpui::test]
fn the_items_of_a_dock_are_saved_and_rebuilt_on_restore(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open(&ws, &mut vcx, "a");
    add_panel(&ws, &mut vcx, DockPosition::Bottom, "terminals", |_| {});
    open_docked(&ws, &mut vcx, "one");
    let two = open_docked(&ws, &mut vcx, "two");
    open_docked(&ws, &mut vcx, "three");
    // Display the middle one.
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.move_item_to_dock(two, DockPosition::Bottom, window, cx)
        })
    });
    vcx.run_until_parked();
    let saved = vcx.update(|_, cx| ws.read(cx).serialize_layout(cx));

    let (fresh, mut fresh_vcx) = workspace(cx);
    let panel = add_panel(
        &fresh,
        &mut fresh_vcx,
        DockPosition::Bottom,
        "terminals",
        |_| {},
    );
    let report = fresh_vcx
        .update(|window, cx| fresh.update(cx, |ws, cx| ws.restore_layout(&saved, window, cx)));
    fresh_vcx.run_until_parked();
    assert_eq!(report.restored_items, 4, "a, and the three docked items");
    assert!(
        report.skipped_items.is_empty(),
        "{:?}",
        report.skipped_items
    );

    let bottom = dock(&fresh, &mut fresh_vcx, DockPosition::Bottom);
    assert_eq!(bottom.panels(), [panel.entity_id()]);
    let titles: Vec<String> = fresh_vcx.update(|_, cx| {
        bottom
            .items()
            .iter()
            .map(|id| {
                fresh
                    .read(cx)
                    .item(*id)
                    .expect("open")
                    .tab_content(cx)
                    .title
                    .to_string()
            })
            .collect()
    });
    assert_eq!(titles, ["one", "two", "three"], "in saved tab order");
    let displayed = bottom.active_item().expect("an item is displayed");
    let title = fresh_vcx.update(|_, cx| {
        fresh
            .read(cx)
            .item(displayed)
            .expect("open")
            .tab_content(cx)
            .title
            .to_string()
    });
    assert_eq!(title, "two", "the displayed tab comes back");
    assert_eq!(
        shape_titles(&fresh, &mut fresh_vcx),
        ["a"],
        "the centre too"
    );
}

#[gpui::test]
fn a_docked_item_that_cannot_be_rebuilt_is_not_saved(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    add_panel(&ws, &mut vcx, DockPosition::Bottom, "terminals", |_| {});
    open_docked(&ws, &mut vcx, "kept");
    let saved = vcx.update(|_, cx| ws.read(cx).serialize_layout(cx));
    let bottom = saved
        .dock_area
        .bottom_dock
        .as_ref()
        .expect("the bottom dock is saved");
    let json = serde_json::to_string(bottom.panel()).expect("json");
    assert!(json.contains("kept"), "{json}");

    // An item that serialises nothing is pruned from the saved dock; the panel stays.
    let (fresh, mut fresh_vcx) = workspace(cx);
    add_panel(
        &fresh,
        &mut fresh_vcx,
        DockPosition::Bottom,
        "terminals",
        |_| {},
    );
    let unsaved = fresh_vcx.update(|window, cx| {
        let item = cx.new(|cx| Unsaved(cx.focus_handle()));
        fresh.update(cx, |ws, cx| {
            ws.open_item_in_dock(Box::new(item), DockPosition::Bottom, true, window, cx)
        })
    });
    assert!(unsaved.is_some());
    let saved = fresh_vcx.update(|_, cx| fresh.read(cx).serialize_layout(cx));
    let bottom = saved
        .dock_area
        .bottom_dock
        .as_ref()
        .expect("the panel keeps the dock");
    let json = serde_json::to_string(bottom.panel()).expect("json");
    assert!(!json.contains(crate::item::ITEM_PANEL_NAME), "{json}");
    assert!(json.contains("TestPanel"), "{json}");
}

/// The titles of the first pane.
fn shape_titles(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> Vec<String> {
    let pane = panes(ws, vcx)[0].clone();
    titles(ws, vcx, &pane)
}

/// A dockable item with nothing to save.
struct Unsaved(gpui::FocusHandle);

impl gpui::EventEmitter<crate::ItemEvent> for Unsaved {}

impl gpui::Focusable for Unsaved {
    fn focus_handle(&self, _: &gpui::App) -> gpui::FocusHandle {
        self.0.clone()
    }
}

impl gpui::Render for Unsaved {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        gpui::div()
    }
}

impl crate::Item for Unsaved {
    fn tab_content(&self, _: &gpui::App) -> crate::TabContent {
        crate::TabContent::new("unsaved")
    }

    fn can_dock(&self, _: &gpui::App) -> bool {
        true
    }
}
