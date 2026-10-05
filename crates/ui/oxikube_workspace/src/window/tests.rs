//! `#[gpui::test]`s of the main window: it opens with the `Root` as its root view, the overlay
//! layers render exactly once, the title bar is drawn, the options per platform, and the menu.

use super::menus::{About, Minimize, OpenPreferences, Quit, app_menus, default_bindings};
use super::options::{APP_ID, Chrome, DEFAULT_SIZE, MIN_SIZE, WINDOW_TITLE, window_options};
use super::*;
use gpui::{
    Bounds, MenuItem, TestAppContext, VisualTestContext, WindowBounds, WindowDecorations, px,
};
use oxikube_ui::{
    dialog::{Dialog, OverlayExt as _},
    title_bar::TITLE_BAR_HEIGHT,
};
use std::{cell::Cell, rc::Rc};

fn open(cx: &mut TestAppContext) -> (WindowHandle<Root>, VisualTestContext) {
    cx.update(|cx| {
        oxikube_ui::init(cx);
        init(cx);
    });
    let handle = cx
        .update(open_main_window)
        .expect("the main window opens in the test platform");
    let vcx = VisualTestContext::from_window(handle.into(), cx);
    vcx.run_until_parked();
    (handle, vcx)
}

#[gpui::test]
fn opens_with_root_hosting_main_view(cx: &mut TestAppContext) {
    let (_handle, mut vcx) = open(cx);
    vcx.update(|window, cx| {
        let root = window
            .root::<Root>()
            .flatten()
            .expect("the window's root view is the Root, or overlays cannot work");
        assert!(root.read(cx).view().clone().downcast::<MainView>().is_ok());
    });
    assert_eq!(vcx.update(|_, cx| cx.windows().len()), 1);
}

#[gpui::test]
fn main_view_hosts_an_empty_workspace(cx: &mut TestAppContext) {
    let (_handle, mut vcx) = open(cx);
    vcx.update(|window, cx| {
        let root = window.root::<Root>().flatten().expect("root");
        let main = root
            .read(cx)
            .view()
            .clone()
            .downcast::<MainView>()
            .expect("main view");
        let workspace = main.read(cx).workspace().read(cx);
        assert!(workspace.is_blank(), "nothing is open at start");
        assert!(workspace.panes(cx).is_empty());
    });
}

#[gpui::test]
fn title_bar_is_drawn_at_the_top(cx: &mut TestAppContext) {
    let (_handle, mut vcx) = open(cx);
    let title = vcx
        .debug_bounds("window-title")
        .expect("the title bar did not render the window title");
    assert!(title.size.width > px(0.) && title.size.height > px(0.));
    assert!(
        title.origin.y >= px(0.) && title.bottom() <= TITLE_BAR_HEIGHT,
        "title {title:?} is outside the {TITLE_BAR_HEIGHT:?} high title bar"
    );
}

#[gpui::test]
fn root_layers_render_exactly_once(cx: &mut TestAppContext) {
    let (_handle, mut vcx) = open(cx);
    let builds = Rc::new(Cell::new(0usize));
    vcx.update(|window, cx| {
        let builds = builds.clone();
        window.open_dialog(cx, move |dialog: Dialog, _, _| {
            // The root calls the builder once per layer it renders per frame.
            builds.set(builds.get() + 1);
            dialog.title("Probe").w(px(320.))
        });
    });
    vcx.run_until_parked();
    assert!(vcx.update(|window, cx| window.has_active_dialog(cx)));
    assert!(
        vcx.debug_bounds("dialog-layer").is_some(),
        "the Root did not render the dialog layer"
    );

    builds.set(0);
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(
        builds.get(),
        1,
        "one frame must render the dialog layer once, not {} times",
        builds.get()
    );
}

#[gpui::test]
fn about_action_opens_a_dialog_and_preferences_does_not_panic(cx: &mut TestAppContext) {
    let (_handle, mut vcx) = open(cx);
    assert!(!vcx.update(|window, cx| window.has_active_dialog(cx)));
    vcx.update(|window, _| window.activate_window());
    assert!(vcx.update(|_, cx| cx.active_window().is_some()));
    vcx.dispatch_action(About);
    vcx.run_until_parked();
    assert!(vcx.update(|window, cx| window.has_active_dialog(cx)));
    vcx.dispatch_action(OpenPreferences);
    vcx.run_until_parked();
}

#[gpui::test]
fn menu_actions_are_registered(cx: &mut TestAppContext) {
    let (_handle, mut vcx) = open(cx);
    vcx.update(|_, cx| {
        assert!(cx.is_action_available(&Quit));
        assert!(cx.is_action_available(&About));
        assert!(cx.is_action_available(&Minimize));
    });
}

#[gpui::test]
fn app_menu_is_installed(cx: &mut TestAppContext) {
    let (_handle, mut vcx) = open(cx);
    let menus = vcx
        .update(|_, cx| cx.get_menus())
        .expect("the platform keeps the menus");
    let names: Vec<_> = menus.iter().map(|m| m.name.to_string()).collect();
    assert_eq!(names, ["Oxikube", "Edit", "Window"]);
}

#[test]
fn app_menu_has_about_preferences_and_quit() {
    let menus = app_menus();
    let items = |ix: usize| -> Vec<String> {
        menus[ix]
            .items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Action { name, .. } => Some(name.to_string()),
                _ => None,
            })
            .collect()
    };
    let app = items(0);
    for wanted in ["About Oxikube", "Preferences…", "Quit Oxikube"] {
        assert!(app.iter().any(|n| n == wanted), "app menu lacks {wanted}");
    }
    let edit = items(1);
    for wanted in ["Undo", "Redo", "Cut", "Copy", "Paste", "Select All"] {
        assert!(edit.iter().any(|n| n == wanted), "Edit menu lacks {wanted}");
    }
    assert_eq!(items(2), ["Minimize", "Zoom"]);
}

#[test]
fn quit_is_bound_on_every_platform() {
    for macos in [true, false] {
        let bindings = default_bindings(macos);
        assert!(
            bindings.iter().any(|b| b.action().partial_eq(&Quit)),
            "no Quit binding (macos: {macos})"
        );
    }
}

fn bounds() -> WindowBounds {
    WindowBounds::Windowed(Bounds::new(gpui::point(px(0.), px(0.)), DEFAULT_SIZE))
}

#[test]
fn linux_requests_client_side_decorations_and_the_app_id() {
    let options = window_options(Chrome::Linux, bounds());
    assert_eq!(options.window_decorations, Some(WindowDecorations::Client));
    assert_eq!(options.app_id.as_deref(), Some(APP_ID));
    assert_eq!(APP_ID, "dev.karan.oxikube");
}

#[test]
fn macos_and_windows_keep_the_native_frame() {
    for chrome in [Chrome::MacOs, Chrome::Windows] {
        let options = window_options(chrome, bounds());
        assert_eq!(options.window_decorations, None, "{chrome:?}");
        let titlebar = options.titlebar.expect("titlebar options");
        assert!(titlebar.appears_transparent, "{chrome:?}");
        // The title bar draws and moves the window itself.
        assert!(options.app_owns_titlebar_drag, "{chrome:?}");
    }
}

#[test]
fn every_platform_titles_and_sizes_the_window() {
    for chrome in [Chrome::MacOs, Chrome::Linux, Chrome::Windows] {
        let options = window_options(chrome, bounds());
        assert_eq!(
            options.titlebar.and_then(|t| t.title).as_deref(),
            Some(WINDOW_TITLE)
        );
        assert_eq!(options.window_min_size, Some(MIN_SIZE));
        assert!(options.show && options.focus);
    }
}

#[gpui::test]
fn layout_is_in_logical_pixels_on_a_hidpi_display(cx: &mut TestAppContext) {
    // The test window reports a 2x scale factor. Layout must stay in logical pixels: the title
    // bar is as high as it is at 1x and not twice that.
    let (_handle, mut vcx) = open(cx);
    assert!(vcx.update(|window, _| window.scale_factor()) > 1.0);
    let title = vcx.debug_bounds("window-title").expect("title rendered");
    assert!(title.size.height <= TITLE_BAR_HEIGHT);
}
