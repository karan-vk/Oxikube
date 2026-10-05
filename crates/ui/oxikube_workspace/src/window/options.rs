//! `WindowOptions` for the main window, per platform.

use gpui::{
    App, Bounds, Pixels, Size, TitlebarOptions, WindowBounds, WindowDecorations, WindowKind,
    WindowOptions, point, px, size,
};
use oxikube_ui::title_bar::TitleBar;

use crate::persistence::{SerializedWindow, restore_window_bounds};

/// The application id. On Linux it is the Wayland `app_id` (and the X11 `WM_CLASS`), which the
/// compositor matches against `dev.karan.oxikube.desktop` to find the icon and group windows, so
/// it must equal the desktop file's name and its `StartupWMClass`
/// (`bins/oxikube/resources/linux/`).
pub const APP_ID: &str = "dev.karan.oxikube";

/// The window title shown by the OS (Mission Control, task switchers, Wayland title).
pub const WINDOW_TITLE: &str = "Oxikube";

/// Size of the first window when there is no saved layout.
pub const DEFAULT_SIZE: Size<Pixels> = size(px(1280.), px(800.));

/// The smallest the window can be resized to.
pub const MIN_SIZE: Size<Pixels> = size(px(640.), px(400.));

/// Which window chrome the window gets. Chosen by the OS; a parameter so the three shapes are
/// testable from any host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chrome {
    /// Native window frame with a transparent title bar: the traffic lights stay native and our
    /// [`TitleBar`] draws underneath them.
    MacOs,
    /// Client-side decorations: we draw the title bar controls, the border, the shadow and the
    /// resize areas (the `Root` does the latter three).
    Linux,
    /// Native frame with a transparent title bar; the min/max/close buttons are ours, mapped to
    /// the native hit-test areas.
    Windows,
}

impl Chrome {
    /// The chrome of the OS this binary runs on.
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Chrome::MacOs
        } else if cfg!(target_os = "windows") {
            Chrome::Windows
        } else {
            Chrome::Linux
        }
    }
}

/// Options for the main window, on the OS this binary runs on, centred on the primary display.
pub fn main_window_options(cx: &App) -> WindowOptions {
    let bounds = Bounds::centered(None, DEFAULT_SIZE, cx);
    window_options(Chrome::current(), WindowBounds::Windowed(bounds))
}

/// Options for the main window opened where a saved layout left it (`saved`, see
/// `persistence::LayoutStore::load`), fitted to the displays that exist now: moved onto a display
/// that is still there, shrunk to fit, kept fully on screen. `None` is the default placement,
/// like [`main_window_options`].
pub fn main_window_options_for(cx: &App, saved: Option<&SerializedWindow>) -> WindowOptions {
    let primary = cx.primary_display().map(|d| d.id());
    let mut displays = cx.displays();
    // The primary display first: that is where an unplaceable window goes.
    displays.sort_by_key(|d| Some(d.id()) != primary);
    let displays: Vec<_> = displays.iter().map(|d| d.bounds()).collect();
    let bounds = restore_window_bounds(saved, &displays, DEFAULT_SIZE);
    window_options(Chrome::current(), bounds)
}

/// Options for the main window with `chrome`.
///
/// Pure (no `App`), so the Linux and Windows shapes are asserted on a macOS host and the other
/// way round. Nothing here touches the disk or the network: the first frame never waits on it.
pub fn window_options(chrome: Chrome, bounds: WindowBounds) -> WindowOptions {
    // The component library's title bar expects a transparent native title bar and does the
    // dragging itself (`app_owns_titlebar_drag`); we only add our title.
    let base = TitleBar::window_options();
    let mut titlebar = base.titlebar.unwrap_or_else(|| TitlebarOptions {
        title: None,
        appears_transparent: true,
        traffic_light_position: Some(point(px(9.), px(9.))),
    });
    titlebar.title = Some(WINDOW_TITLE.into());
    WindowOptions {
        window_bounds: Some(bounds),
        titlebar: Some(titlebar),
        kind: WindowKind::Normal,
        focus: true,
        show: true,
        window_min_size: Some(MIN_SIZE),
        app_id: Some(APP_ID.to_owned()),
        window_decorations: (chrome == Chrome::Linux).then_some(WindowDecorations::Client),
        ..base
    }
}
