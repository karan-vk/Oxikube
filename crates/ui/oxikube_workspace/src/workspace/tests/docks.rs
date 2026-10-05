//! Docks, side panels, `toggle_panel` focus handling, and zoom.

use gpui::px;
use oxikube_ui::Unscaled;

use super::*;
use crate::{
    PanelEvent, SplitDirection,
    actions::{ToggleBottomDock, ToggleLeftDock, ToggleRightDock, ToggleZoom},
    test_support::ToggleLeftTestPanel,
};

#[gpui::test]
fn panels_create_their_docks_and_each_dock_toggles(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open(&ws, &mut vcx, "a");
    let left = add_panel(&ws, &mut vcx, DockPosition::Left, "left", |_| {});
    let bottom = add_panel(&ws, &mut vcx, DockPosition::Bottom, "bottom", |_| {});
    let right = add_panel(&ws, &mut vcx, DockPosition::Right, "right", |_| {});

    for (position, panel, selector, action) in [
        (
            DockPosition::Left,
            &left,
            "panel-left",
            &ToggleLeftDock as &dyn gpui::Action,
        ),
        (
            DockPosition::Bottom,
            &bottom,
            "panel-bottom",
            &ToggleBottomDock,
        ),
        (DockPosition::Right, &right, "panel-right", &ToggleRightDock),
    ] {
        let snapshot = dock(&ws, &mut vcx, position);
        assert!(
            snapshot.is_open(),
            "{position:?} opens with its first panel"
        );
        assert_eq!(snapshot.panels(), [panel.entity_id()]);
        assert_eq!(snapshot.active_panel(), Some(panel.entity_id()));
        assert_eq!(
            snapshot.size(),
            Unscaled(240.),
            "{position:?} default size, unscaled"
        );
        assert!(bounds(&mut vcx, selector).is_some(), "{selector} is drawn");

        vcx.update(|window, cx| window.dispatch_action(action.boxed_clone(), cx));
        vcx.run_until_parked();
        assert!(
            !dock(&ws, &mut vcx, position).is_open(),
            "{position:?} closed"
        );
        assert!(bounds(&mut vcx, selector).is_none(), "{selector} hidden");
        assert!(bounds(&mut vcx, "item-a").is_some(), "the centre stays");

        let open =
            vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.toggle_dock(position, window, cx)));
        vcx.run_until_parked();
        assert!(open && dock(&ws, &mut vcx, position).is_open());
        assert!(
            panel_focused(&mut vcx, panel),
            "opening a dock focuses its panel"
        );
    }
}

#[gpui::test]
fn toggle_panel_shows_focuses_and_hides(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let a = open(&ws, &mut vcx, "a");
    let panel = add_panel(&ws, &mut vcx, DockPosition::Left, "logs", |_| {});
    // Start with the dock closed and the item focused.
    vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.toggle_dock(DockPosition::Left, window, cx)));
    vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.activate_item(a, true, window, cx)));
    vcx.run_until_parked();
    assert!(!dock(&ws, &mut vcx, DockPosition::Left).is_open());

    let toggle = |ws: &Entity<Workspace>, vcx: &mut VisualTestContext| {
        let shown = vcx
            .update(|window, cx| ws.update(cx, |ws, cx| ws.toggle_panel::<TestPanel>(window, cx)));
        vcx.run_until_parked();
        shown
    };

    // Hidden: open, display, focus.
    assert!(toggle(&ws, &mut vcx));
    assert!(dock(&ws, &mut vcx, DockPosition::Left).is_open());
    assert!(panel_focused(&mut vcx, &panel));
    assert!(vcx.update(|_, cx| panel.read(cx).active));

    // Shown and focused: close, focus back to the centre.
    assert!(!toggle(&ws, &mut vcx));
    assert!(!dock(&ws, &mut vcx, DockPosition::Left).is_open());
    assert!(item_focused(&ws, &mut vcx, a));

    // Shown but not focused: just focus it.
    assert!(toggle(&ws, &mut vcx));
    vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.activate_item(a, true, window, cx)));
    vcx.run_until_parked();
    assert!(toggle(&ws, &mut vcx));
    assert!(dock(&ws, &mut vcx, DockPosition::Left).is_open());
    assert!(panel_focused(&mut vcx, &panel));

    // The panel's own toggle action does the same.
    vcx.dispatch_action(ToggleLeftTestPanel);
    vcx.run_until_parked();
    assert!(!dock(&ws, &mut vcx, DockPosition::Left).is_open());
    assert!(item_focused(&ws, &mut vcx, a));
}

#[gpui::test]
fn toggle_panel_displays_a_background_tab(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open(&ws, &mut vcx, "a");
    let first = add_panel(&ws, &mut vcx, DockPosition::Bottom, "events", |p| {
        p.priority = 2
    });
    let second = add_panel(&ws, &mut vcx, DockPosition::Bottom, "logs", |p| {
        p.priority = 1
    });
    let snapshot = dock(&ws, &mut vcx, DockPosition::Bottom);
    assert_eq!(
        snapshot.panels(),
        [second.entity_id(), first.entity_id()],
        "ordered by activation priority"
    );
    assert_eq!(
        snapshot.active_panel(),
        Some(first.entity_id()),
        "the displayed tab stays"
    );
    assert!(
        bounds(&mut vcx, "panel-tab-logs").is_some(),
        "both tabs are drawn"
    );

    let shown = vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.toggle_panel_by_id(second.entity_id(), window, cx)
        })
    });
    vcx.run_until_parked();
    assert!(shown);
    assert_eq!(
        dock(&ws, &mut vcx, DockPosition::Bottom).active_panel(),
        Some(second.entity_id())
    );
    assert!(panel_focused(&mut vcx, &second));
    assert!(bounds(&mut vcx, "panel-logs").is_some());
    assert!(bounds(&mut vcx, "panel-events").is_none());
}

#[gpui::test]
fn panel_events_activate_and_close_the_dock(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open(&ws, &mut vcx, "a");
    let panel = add_panel(&ws, &mut vcx, DockPosition::Right, "agent", |_| {});
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| ws.toggle_dock(DockPosition::Right, window, cx))
    });
    vcx.run_until_parked();
    assert!(!dock(&ws, &mut vcx, DockPosition::Right).is_open());

    vcx.update(|_, cx| panel.update(cx, |panel, cx| panel.activate(cx)));
    vcx.run_until_parked();
    assert!(dock(&ws, &mut vcx, DockPosition::Right).is_open());
    assert!(panel_focused(&mut vcx, &panel));

    vcx.update(|_, cx| panel.update(cx, |_, cx| cx.emit(PanelEvent::Close)));
    vcx.run_until_parked();
    assert!(!dock(&ws, &mut vcx, DockPosition::Right).is_open());
}

#[gpui::test]
fn dock_resizes_respect_the_panel_min_size(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    add_panel(&ws, &mut vcx, DockPosition::Left, "tree", |p| {
        p.min_size = Some(px(180.))
    });
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.resize_dock(DockPosition::Left, Unscaled(320.), window, cx)
        })
    });
    vcx.run_until_parked();
    assert_eq!(
        dock(&ws, &mut vcx, DockPosition::Left).size(),
        Unscaled(320.)
    );
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.resize_dock(DockPosition::Left, Unscaled(100.), window, cx)
        })
    });
    vcx.run_until_parked();
    assert_eq!(
        dock(&ws, &mut vcx, DockPosition::Left).size(),
        Unscaled(180.)
    );
}

#[gpui::test]
fn zoom_and_unzoom_the_active_pane(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open(&ws, &mut vcx, "a");
    let b = open(&ws, &mut vcx, "b");
    add_panel(&ws, &mut vcx, DockPosition::Left, "tree", |_| {});
    let right = vcx
        .update(|window, cx| {
            ws.update(cx, |ws, cx| {
                ws.move_item_to_split(b, SplitDirection::Right, window, cx)
            })
        })
        .expect("split");
    vcx.run_until_parked();
    assert!(bounds(&mut vcx, "item-a").is_some() && bounds(&mut vcx, "panel-tree").is_some());

    vcx.dispatch_action(ToggleZoom);
    vcx.run_until_parked();
    assert!(vcx.update(|_, cx| ws.read(cx).is_zoomed(cx)));
    assert_eq!(vcx.update(|_, cx| ws.read(cx).zoomed_pane(cx)), Some(right));
    assert!(
        bounds(&mut vcx, "item-b").is_some(),
        "the zoomed pane fills the window"
    );
    assert!(
        bounds(&mut vcx, "item-a").is_none(),
        "other panes are hidden"
    );
    assert!(bounds(&mut vcx, "panel-tree").is_none(), "docks are hidden");

    vcx.dispatch_action(ToggleZoom);
    vcx.run_until_parked();
    assert!(!vcx.update(|_, cx| ws.read(cx).is_zoomed(cx)));
    assert!(bounds(&mut vcx, "item-a").is_some() && bounds(&mut vcx, "panel-tree").is_some());
}

#[gpui::test]
fn zoom_a_focused_panel(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open(&ws, &mut vcx, "a");
    let panel = add_panel(&ws, &mut vcx, DockPosition::Bottom, "logs", |_| {});
    vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.toggle_panel::<TestPanel>(window, cx)));
    vcx.run_until_parked();
    assert!(panel_focused(&mut vcx, &panel));

    vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.toggle_zoom(window, cx)));
    vcx.run_until_parked();
    assert!(vcx.update(|_, cx| ws.read(cx).is_zoomed(cx)));
    assert_eq!(
        vcx.update(|_, cx| ws.read(cx).zoomed_pane(cx)),
        None,
        "a dock group, not a pane"
    );
    assert!(
        vcx.update(|_, cx| panel.read(cx).zoomed),
        "the panel is told"
    );
    assert!(bounds(&mut vcx, "item-a").is_none());

    vcx.update(|_, cx| panel.update(cx, |_, cx| cx.emit(PanelEvent::ZoomOut)));
    vcx.run_until_parked();
    assert!(!vcx.update(|_, cx| ws.read(cx).is_zoomed(cx)));
    assert!(!vcx.update(|_, cx| panel.read(cx).zoomed));
}
