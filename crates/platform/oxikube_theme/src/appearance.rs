//! Light or dark, and the operating system's current choice.

use gpui::{App, Global, Subscription, Window, WindowAppearance};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Light or dark: the two halves of a theme family.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    /// Light surfaces, dark text.
    Light,
    /// Dark surfaces, light text.
    #[default]
    Dark,
}

impl Appearance {
    /// Whether this is [`Appearance::Dark`].
    pub fn is_dark(self) -> bool {
        self == Appearance::Dark
    }
}

impl From<WindowAppearance> for Appearance {
    fn from(value: WindowAppearance) -> Self {
        match value {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Appearance::Light,
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Appearance::Dark,
        }
    }
}

/// How the `theme` setting picks between a light and a dark theme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    /// Follow the operating system's appearance.
    #[default]
    System,
    /// Always use the light theme.
    Light,
    /// Always use the dark theme.
    Dark,
}

impl ThemeMode {
    /// The appearance this mode asks for, given what the system currently shows.
    pub fn appearance(self, system: Appearance) -> Appearance {
        match self {
            ThemeMode::System => system,
            ThemeMode::Light => Appearance::Light,
            ThemeMode::Dark => Appearance::Dark,
        }
    }
}

/// The operating system's current appearance, as a GPUI global.
///
/// Set once by [`SystemAppearance::init`] and kept current by [`SystemAppearance::follow`].
/// Tests (and previews) pin it with [`SystemAppearance::set`] instead of a real OS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemAppearance(pub Appearance);

impl Global for SystemAppearance {}

impl SystemAppearance {
    /// Installs the global from the platform's current appearance (a no-op when set already).
    pub fn init(cx: &mut App) {
        if !cx.has_global::<Self>() {
            let appearance = Appearance::from(cx.window_appearance());
            cx.set_global(Self(appearance));
        }
    }

    /// The system appearance; dark until [`SystemAppearance::init`] has run.
    pub fn get(cx: &App) -> Appearance {
        cx.try_global::<Self>()
            .map(|global| global.0)
            .unwrap_or_default()
    }

    /// Overrides the recorded appearance (tests and previews). Observers of the global run when
    /// the value changes.
    pub fn set(cx: &mut App, appearance: Appearance) {
        if cx.try_global::<Self>().map(|global| global.0) != Some(appearance) {
            cx.set_global(Self(appearance));
        }
    }

    /// Keeps the global in step with `window`'s appearance: the platform tells the window when
    /// the user flips light/dark. Drop the returned subscription to stop following.
    pub fn follow(window: &mut Window, cx: &mut App) -> Subscription {
        Self::set(cx, Appearance::from(window.appearance()));
        window.observe_window_appearance(|window, cx| {
            Self::set(cx, Appearance::from(window.appearance()));
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_resolves_against_the_system_appearance() {
        assert_eq!(
            ThemeMode::System.appearance(Appearance::Light),
            Appearance::Light
        );
        assert_eq!(
            ThemeMode::System.appearance(Appearance::Dark),
            Appearance::Dark
        );
        assert_eq!(
            ThemeMode::Light.appearance(Appearance::Dark),
            Appearance::Light
        );
        assert_eq!(
            ThemeMode::Dark.appearance(Appearance::Light),
            Appearance::Dark
        );
    }

    #[test]
    fn window_appearance_maps_vibrancy_variants() {
        assert_eq!(
            Appearance::from(WindowAppearance::VibrantLight),
            Appearance::Light
        );
        assert_eq!(
            Appearance::from(WindowAppearance::VibrantDark),
            Appearance::Dark
        );
    }

    #[gpui::test]
    fn set_overrides_and_observers_see_changes(cx: &mut gpui::TestAppContext) {
        use std::{cell::Cell, rc::Rc};
        let seen = Rc::new(Cell::new(0));
        cx.update(|cx| {
            SystemAppearance::init(cx);
            SystemAppearance::set(cx, Appearance::Light);
            let counter = seen.clone();
            cx.observe_global::<SystemAppearance>(move |_| counter.set(counter.get() + 1))
                .detach();
        });
        cx.update(|cx| SystemAppearance::set(cx, Appearance::Light));
        assert_eq!(seen.get(), 0, "an unchanged value must not notify");
        cx.update(|cx| SystemAppearance::set(cx, Appearance::Dark));
        assert_eq!(seen.get(), 1);
        cx.update(|cx| assert_eq!(SystemAppearance::get(cx), Appearance::Dark));
    }
}
