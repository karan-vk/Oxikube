//! Dock sizes: the panel's minimum size on the divider-drag path, and sizes kept unscaled at a UI
//! zoom other than 100 %.

use gpui::{Modifiers, MouseButton, point, px, size};
use oxikube_ui::{UiScale, Unscaled, dock::DockPlacement, set_ui_scale};

use super::*;

/// The dock area's own (scaled) size of the dock at `position`.
fn scaled_size(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    position: DockPosition,
) -> Option<Pixels> {
    vcx.update(|_, cx| {
        ws.read(cx)
            .dock_area()
            .read(cx)
            .dock_size(position.placement())
    })
}

/// Resizes the dock at `position` through the dock area, as the divider does, bypassing
/// [`Workspace::resize_dock`].
fn set_area_dock_size(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    placement: DockPlacement,
    size: Pixels,
) {
    vcx.update(|window, cx| {
        let area = ws.read(cx).dock_area().clone();
        area.update(cx, |area, cx| {
            area.set_dock_size(placement, size, window, cx)
        });
    });
    vcx.run_until_parked();
}

#[gpui::test]
fn dragging_the_dock_divider_below_the_panel_min_size_settles_at_the_min_size(
    cx: &mut TestAppContext,
) {
    let (ws, mut vcx) = workspace(cx);
    vcx.simulate_resize(size(px(1000.), px(700.)));
    open(&ws, &mut vcx, "a");
    add_panel(&ws, &mut vcx, DockPosition::Left, "tree", |p| {
        p.min_size = Some(px(180.))
    });
    assert_eq!(
        dock(&ws, &mut vcx, DockPosition::Left).size(),
        Unscaled(240.)
    );

    // The divider sits on the dock's trailing edge, inside the dock.
    let panel = bounds(&mut vcx, "panel-tree").expect("the panel is drawn");
    let left = panel.origin.x;
    let edge = left + panel.size.width;
    let y = panel.origin.y + panel.size.height / 2.;
    let handle = point(edge - px(1.5), y);
    vcx.simulate_mouse_down(handle, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(
        point(handle.x + px(6.), y),
        MouseButton::Left,
        Modifiers::none(),
    );
    // Above the dock area's own floor, below the panel's minimum.
    let release = point(left + px(130.), y);
    vcx.simulate_mouse_move(release, MouseButton::Left, Modifiers::none());
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    vcx.simulate_mouse_up(release, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();

    // The dock area only reports a divider drag on release; the workspace clamps it then.
    let dock = dock(&ws, &mut vcx, DockPosition::Left);
    assert!(dock.is_open());
    assert_eq!(dock.size(), Unscaled(180.));
    assert_eq!(
        scaled_size(&ws, &mut vcx, DockPosition::Left),
        Some(px(180.))
    );
}

#[gpui::test]
fn a_dock_resized_through_the_dock_area_keeps_the_panel_min_size(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    add_panel(&ws, &mut vcx, DockPosition::Bottom, "logs", |p| {
        p.min_size = Some(px(150.))
    });

    set_area_dock_size(&ws, &mut vcx, DockPlacement::Bottom, px(120.));
    assert_eq!(
        dock(&ws, &mut vcx, DockPosition::Bottom).size(),
        Unscaled(150.)
    );

    set_area_dock_size(&ws, &mut vcx, DockPlacement::Bottom, px(300.));
    assert_eq!(
        dock(&ws, &mut vcx, DockPosition::Bottom).size(),
        Unscaled(300.),
        "bigger than the minimum is left alone"
    );
}

#[gpui::test]
fn dock_sizes_are_scaled_on_screen_and_unscaled_in_the_model(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    vcx.update(|_, cx| set_ui_scale(cx, UiScale::new(1.5)));
    add_panel(&ws, &mut vcx, DockPosition::Left, "tree", |p| {
        p.min_size = Some(px(180.))
    });

    // Opened at the panel's default size, 240 unscaled.
    assert_eq!(
        scaled_size(&ws, &mut vcx, DockPosition::Left),
        Some(px(360.))
    );
    assert_eq!(
        dock(&ws, &mut vcx, DockPosition::Left).size(),
        Unscaled(240.)
    );

    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.resize_dock(DockPosition::Left, Unscaled(320.), window, cx)
        })
    });
    vcx.run_until_parked();
    assert_eq!(
        scaled_size(&ws, &mut vcx, DockPosition::Left),
        Some(px(480.))
    );
    assert_eq!(
        dock(&ws, &mut vcx, DockPosition::Left).size(),
        Unscaled(320.)
    );

    // The minimum is unscaled too: 180 unscaled is 270 on screen.
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.resize_dock(DockPosition::Left, Unscaled(100.), window, cx)
        })
    });
    vcx.run_until_parked();
    assert_eq!(
        scaled_size(&ws, &mut vcx, DockPosition::Left),
        Some(px(270.))
    );
    assert_eq!(
        dock(&ws, &mut vcx, DockPosition::Left).size(),
        Unscaled(180.)
    );

    // And on the divider path: 150 on screen (100 unscaled) is below it.
    set_area_dock_size(&ws, &mut vcx, DockPlacement::Left, px(150.));
    assert_eq!(
        scaled_size(&ws, &mut vcx, DockPosition::Left),
        Some(px(270.))
    );
    assert_eq!(
        dock(&ws, &mut vcx, DockPosition::Left).size(),
        Unscaled(180.)
    );
}
