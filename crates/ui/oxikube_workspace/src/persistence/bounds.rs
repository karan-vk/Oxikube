//! Restoring window bounds onto the displays that exist now.
//!
//! A layout saved on a docked laptop with an external monitor may be restored on the bare
//! laptop, on a different resolution, or with the monitor on the other side. The saved bounds are
//! therefore never trusted: they go to the display they overlap most, are clamped into it, and
//! fall back to the primary display when no display holds the window any more.

use gpui::{Bounds, Pixels, Size, WindowBounds, point, px, size};

use super::model::{SerializedWindow, WindowMode};
use crate::window::options::MIN_SIZE;

impl SerializedWindow {
    /// The bounds as saved.
    pub fn bounds(&self) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(self.x), px(self.y)),
            size: size(px(self.width), px(self.height)),
        }
    }

    /// What to store for a window currently at `bounds`.
    pub fn from_window_bounds(bounds: WindowBounds) -> Self {
        let (mode, b) = match bounds {
            WindowBounds::Windowed(b) => (WindowMode::Windowed, b),
            WindowBounds::Maximized(b) => (WindowMode::Maximized, b),
            WindowBounds::Fullscreen(b) => (WindowMode::Fullscreen, b),
        };
        Self {
            mode,
            x: f32::from(b.origin.x),
            y: f32::from(b.origin.y),
            width: f32::from(b.size.width),
            height: f32::from(b.size.height),
        }
    }
}

fn area(b: Bounds<Pixels>) -> f32 {
    f32::from(b.size.width.max(px(0.))) * f32::from(b.size.height.max(px(0.)))
}

/// Pulls `bounds` fully inside `display`, shrinking it to the display (but not below
/// [`MIN_SIZE`]) when it is bigger.
pub fn clamp_into(bounds: Bounds<Pixels>, display: Bounds<Pixels>) -> Bounds<Pixels> {
    let width = bounds
        .size
        .width
        .min(display.size.width)
        .max(MIN_SIZE.width.min(display.size.width));
    let height = bounds
        .size
        .height
        .min(display.size.height)
        .max(MIN_SIZE.height.min(display.size.height));
    let max_x = display.origin.x + display.size.width - width;
    let max_y = display.origin.y + display.size.height - height;
    Bounds {
        origin: point(
            bounds
                .origin
                .x
                .clamp(display.origin.x, max_x.max(display.origin.x)),
            bounds
                .origin
                .y
                .clamp(display.origin.y, max_y.max(display.origin.y)),
        ),
        size: size(width, height),
    }
}

/// The window bounds to open with: `saved` fitted to `displays` (the first one is the primary
/// display), or `default_size` centred on the primary display when nothing was saved. Non-finite
/// or empty saved sizes count as nothing saved.
///
/// Pure: pass the displays' bounds in (`cx.displays()`); the result goes to
/// `WindowOptions::window_bounds`.
pub fn restore_window_bounds(
    saved: Option<&SerializedWindow>,
    displays: &[Bounds<Pixels>],
    default_size: Size<Pixels>,
) -> WindowBounds {
    let Some(primary) = displays.first().copied() else {
        // No display information (headless): the size is all that can be honoured.
        return WindowBounds::Windowed(Bounds {
            origin: point(px(0.), px(0.)),
            size: default_size,
        });
    };
    let centred = |size: Size<Pixels>| {
        clamp_into(
            Bounds {
                origin: point(
                    primary.origin.x + (primary.size.width - size.width) / 2.,
                    primary.origin.y + (primary.size.height - size.height) / 2.,
                ),
                size,
            },
            primary,
        )
    };
    let Some(saved) = saved.filter(|s| {
        [s.x, s.y, s.width, s.height].iter().all(|v| v.is_finite()) && s.width > 0. && s.height > 0.
    }) else {
        return WindowBounds::Windowed(centred(default_size));
    };
    let wanted = saved.bounds();
    let best = displays
        .iter()
        .copied()
        .map(|d| (area(wanted.intersect(&d)), d))
        .filter(|(overlap, _)| *overlap > 0.)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, d)| d);
    let bounds = match best {
        Some(display) => clamp_into(wanted, display),
        // Its display is gone: keep the size, move it onto the primary display.
        None => centred(clamp_into(wanted, primary).size),
    };
    match saved.mode {
        WindowMode::Windowed => WindowBounds::Windowed(bounds),
        WindowMode::Maximized => WindowBounds::Maximized(bounds),
        WindowMode::Fullscreen => WindowBounds::Fullscreen(bounds),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(x), px(y)),
            size: size(px(w), px(h)),
        }
    }

    fn saved(mode: WindowMode, x: f32, y: f32, w: f32, h: f32) -> SerializedWindow {
        SerializedWindow {
            mode,
            x,
            y,
            width: w,
            height: h,
        }
    }

    fn bounds_of(b: WindowBounds) -> Bounds<Pixels> {
        match b {
            WindowBounds::Windowed(b)
            | WindowBounds::Maximized(b)
            | WindowBounds::Fullscreen(b) => b,
        }
    }

    const DEFAULT: Size<Pixels> = size(px(1280.), px(800.));

    #[test]
    fn bounds_that_still_fit_are_kept() {
        let displays = [display(0., 0., 1920., 1080.)];
        let got = restore_window_bounds(
            Some(&saved(WindowMode::Windowed, 100., 50., 900., 700.)),
            &displays,
            DEFAULT,
        );
        assert_eq!(got, WindowBounds::Windowed(display(100., 50., 900., 700.)));
    }

    #[test]
    fn nothing_saved_centres_the_default_on_the_primary_display() {
        let displays = [display(0., 0., 2000., 1000.)];
        let got = restore_window_bounds(None, &displays, DEFAULT);
        assert_eq!(
            got,
            WindowBounds::Windowed(display(360., 100., 1280., 800.))
        );
    }

    #[test]
    fn a_window_from_a_missing_second_monitor_moves_to_the_primary() {
        let displays = [display(0., 0., 1440., 900.)];
        let got = restore_window_bounds(
            Some(&saved(WindowMode::Windowed, 2500., 100., 1000., 700.)),
            &displays,
            DEFAULT,
        );
        let b = bounds_of(got);
        assert_eq!(b.size, size(px(1000.), px(700.)));
        assert_eq!(b.intersect(&displays[0]), b, "fully on the primary: {b:?}");
    }

    #[test]
    fn a_window_bigger_than_the_new_display_is_shrunk_to_it() {
        let displays = [display(0., 0., 1024., 768.)];
        let got = restore_window_bounds(
            Some(&saved(WindowMode::Windowed, 0., 0., 2560., 1440.)),
            &displays,
            DEFAULT,
        );
        assert_eq!(bounds_of(got), displays[0]);
    }

    #[test]
    fn a_partly_off_screen_window_is_pulled_back_in() {
        let displays = [display(0., 0., 1920., 1080.)];
        let got = restore_window_bounds(
            Some(&saved(WindowMode::Windowed, 1700., -80., 800., 600.)),
            &displays,
            DEFAULT,
        );
        assert_eq!(bounds_of(got), display(1120., 0., 800., 600.));
    }

    #[test]
    fn the_display_with_the_most_overlap_wins() {
        let displays = [
            display(0., 0., 1920., 1080.),
            display(1920., 0., 1920., 1080.),
        ];
        let got = restore_window_bounds(
            Some(&saved(WindowMode::Windowed, 1800., 100., 1000., 700.)),
            &displays,
            DEFAULT,
        );
        // 880 px of it is on the second display, 120 on the first.
        assert_eq!(bounds_of(got), display(1920., 100., 1000., 700.));
    }

    #[test]
    fn the_saved_mode_survives_and_garbage_does_not() {
        let displays = [display(0., 0., 1920., 1080.)];
        let maximised = restore_window_bounds(
            Some(&saved(WindowMode::Maximized, 10., 10., 900., 700.)),
            &displays,
            DEFAULT,
        );
        assert!(matches!(maximised, WindowBounds::Maximized(_)));
        for bad in [
            saved(WindowMode::Windowed, f32::NAN, 0., 900., 700.),
            saved(WindowMode::Windowed, 0., 0., 0., 700.),
            saved(WindowMode::Windowed, 0., 0., f32::INFINITY, 700.),
        ] {
            let got = restore_window_bounds(Some(&bad), &displays, DEFAULT);
            assert_eq!(bounds_of(got).size, DEFAULT, "{bad:?}");
        }
    }

    #[test]
    fn a_tiny_saved_size_is_raised_to_the_minimum() {
        let displays = [display(0., 0., 1920., 1080.)];
        let got = restore_window_bounds(
            Some(&saved(WindowMode::Windowed, 0., 0., 100., 100.)),
            &displays,
            DEFAULT,
        );
        assert_eq!(bounds_of(got).size, MIN_SIZE);
    }

    #[test]
    fn no_displays_means_the_default_size() {
        let got = restore_window_bounds(
            Some(&saved(WindowMode::Windowed, 5., 5., 900., 700.)),
            &[],
            DEFAULT,
        );
        assert_eq!(bounds_of(got).size, DEFAULT);
    }

    #[test]
    fn window_bounds_round_trip_through_the_saved_form() {
        let b = WindowBounds::Fullscreen(display(1., 2., 3., 4.));
        let saved = SerializedWindow::from_window_bounds(b);
        assert_eq!(saved.mode, WindowMode::Fullscreen);
        assert_eq!(saved.bounds(), display(1., 2., 3., 4.));
    }
}
