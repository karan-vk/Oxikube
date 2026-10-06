//! [`ClusterMark`]: what a cluster looks like at a glance, and [`ClusterBadge`], which draws it.

use gpui::{
    App, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, RenderOnce,
    SharedString, Styled as _, Window, div, prelude::*, px,
};
use oxikube_app::ClusterSession;
use oxikube_domain::ClusterColour;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, layout::h_flex, u};

use super::colour::badge_colour;

/// The two things a badge shows about a cluster: its colour and whether it is read-only.
///
/// The read-only lock is separate from the colour (a read-only cluster without a colour still
/// shows the lock; a colour without read-only shows only the dot), so each can be read at a
/// glance and neither is hidden state of the other.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClusterMark {
    /// The user's accent colour, if any.
    pub colour: Option<ClusterColour>,
    /// Whether mutations are blocked.
    pub read_only: bool,
}

impl ClusterMark {
    /// The mark of a session snapshot.
    pub fn of(session: &ClusterSession) -> Self {
        Self {
            colour: session.colour(),
            read_only: session.read_only(),
        }
    }

    /// Whether there is nothing to draw.
    pub fn is_plain(&self) -> bool {
        self.colour.is_none() && !self.read_only
    }
}

/// Where a badge is drawn; sizes the dot and the lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeSurface {
    /// A cluster tab in the centre pane.
    Tab,
    /// A cluster entry in the hotbar.
    Hotbar,
    /// The status bar item.
    StatusBar,
}

impl BadgeSurface {
    /// `(dot, lock)` edge lengths in unscaled pixels.
    const fn sizes(self) -> (Pixels, Pixels) {
        match self {
            BadgeSurface::Tab => (px(8.), px(12.)),
            BadgeSurface::Hotbar => (px(10.), px(14.)),
            BadgeSurface::StatusBar => (px(8.), px(12.)),
        }
    }
}

/// A colour dot and, when read-only, a lock icon in the cluster's colour.
///
/// One element for every place a cluster is badged (tab, hotbar, status bar), so they cannot
/// drift apart. `name` prefixes the `debug_selector`s (`<name>-dot`, `<name>-lock`) so tests find
/// each one. A plain mark draws nothing.
#[derive(IntoElement)]
pub struct ClusterBadge {
    mark: ClusterMark,
    surface: BadgeSurface,
    name: SharedString,
}

impl ClusterBadge {
    /// A badge for `mark` on `surface`.
    pub fn new(mark: ClusterMark, surface: BadgeSurface) -> Self {
        Self {
            mark,
            surface,
            name: "badge".into(),
        }
    }

    /// Sets the selector prefix.
    pub fn name(mut self, name: impl Into<SharedString>) -> Self {
        self.name = name.into();
        self
    }
}

impl RenderOnce for ClusterBadge {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = cx.tokens();
        let colour = self.mark.colour.map(|c| badge_colour(c, &tokens));
        let (dot, lock) = self.surface.sizes();
        let dot_selector = format!("{}-dot", self.name);
        let lock_selector = format!("{}-lock", self.name);
        let lock_colour = colour.unwrap_or(tokens.colors.text_muted);
        h_flex()
            .flex_none()
            .items_center()
            .gap(u(tokens.spacing.xs))
            .when_some(colour, |this, colour| {
                this.child(
                    div()
                        .debug_selector(move || dot_selector)
                        .flex_none()
                        .size(u(dot))
                        .rounded_full()
                        .bg(colour),
                )
            })
            .when(self.mark.read_only, |this| {
                this.child(
                    div()
                        .debug_selector(move || lock_selector)
                        .flex_none()
                        .child(Icon::new(IconName::Lock).size(u(lock)).color(lock_colour)),
                )
            })
    }
}
