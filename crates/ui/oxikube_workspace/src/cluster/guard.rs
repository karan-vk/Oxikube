//! [`GuardStatusItem`]: the active cluster's guard mode in the right of the status bar.

use std::rc::Rc;

use gpui::{
    App, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Task, Window, div, prelude::*, px,
};
use oxikube_app::{ClusterSession, ClusterSessionManager};
use oxikube_domain::{ClusterColour, command::Command, ids::ClusterId};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, layout::h_flex, tooltip::Tooltip, u};

use super::colour::{badge_colour, label_colour};
use super::follow::follow_session;
use crate::status_bar::StatusItem;

/// What the guard does for a cluster right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardMode {
    /// Every mutation is refused.
    ReadOnly,
    /// Mutations are allowed, behind the confirmation tiers and the audit log.
    Guarded,
}

impl GuardMode {
    /// The mode of a cluster whose read-only flag is `read_only`.
    pub const fn of(read_only: bool) -> Self {
        if read_only {
            Self::ReadOnly
        } else {
            Self::Guarded
        }
    }

    /// The word shown in the status bar.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "Read-only",
            Self::Guarded => "Guarded",
        }
    }

    /// What the mode blocks and what a click does, for the tooltip.
    pub const fn tooltip(self) -> &'static str {
        match self {
            Self::ReadOnly => {
                "Read-only: create, edit, delete, scale, restart, drain and node shell are \
                 blocked for every initiator, and pod shells too. Click to allow changes."
            }
            Self::Guarded => {
                "Guarded: changes are allowed, but destructive ones ask for confirmation and \
                 every change is audited. Click to make this cluster read-only."
            }
        }
    }

    /// The command a click dispatches for `cluster`.
    pub fn toggle(self, cluster: ClusterId) -> Command {
        Command::ClusterToggleReadOnly {
            cluster,
            read_only: Some(self != Self::ReadOnly),
        }
    }
}

/// Runs a command the item builds on click; normally
/// [`ClusterCommandRunner::run`](super::ClusterCommandRunner::run).
pub type RunCommand = Rc<dyn Fn(Command, &mut Window, &mut App)>;

/// The status bar's guard-mode indicator ("Read-only" with a lock, or "Guarded").
///
/// Follows the session by itself ([`follow_session`]), so a flag or colour change redraws just
/// this item, once. A click dispatches `cluster::ToggleReadOnly` through the runner given to
/// [`on_toggle`](Self::on_toggle); the guard, not this item, decides whether that is allowed or
/// needs a confirmation. Register it on the right with
/// [`Workspace::register_status_item`](crate::Workspace::register_status_item) and set the
/// cluster with [`set_cluster`](Self::set_cluster).
pub struct GuardStatusItem {
    manager: ClusterSessionManager,
    cluster: Option<ClusterId>,
    mode: GuardMode,
    colour: Option<ClusterColour>,
    run: Option<RunCommand>,
    _follow: Task<()>,
}

impl GuardStatusItem {
    /// An item with no active cluster, following `manager`.
    pub fn new(manager: ClusterSessionManager, cx: &mut Context<Self>) -> Self {
        let follow = follow_session(
            &manager,
            cx,
            |this: &Self| this.cluster.clone(),
            |this, session, cx| {
                this.show(session);
                cx.notify();
            },
        );
        Self {
            manager,
            cluster: None,
            mode: GuardMode::Guarded,
            colour: None,
            run: None,
            _follow: follow,
        }
    }

    /// Sets what a click runs.
    #[must_use]
    pub fn on_toggle(mut self, run: RunCommand) -> Self {
        self.run = Some(run);
        self
    }

    /// Makes `cluster` the one shown (`None` hides the item).
    pub fn set_cluster(&mut self, cluster: Option<ClusterId>, cx: &mut Context<Self>) {
        let session = cluster.as_ref().and_then(|id| self.manager.get(id));
        self.cluster = cluster;
        self.show(session);
        cx.notify();
    }

    fn show(&mut self, session: Option<ClusterSession>) {
        self.mode = GuardMode::of(session.as_ref().is_some_and(ClusterSession::read_only));
        self.colour = session.and_then(|s| s.colour());
    }

    /// The mode shown.
    pub fn mode(&self) -> GuardMode {
        self.mode
    }
}

impl StatusItem for GuardStatusItem {
    fn visible(&self, _: &App) -> bool {
        self.cluster.is_some()
    }
}

impl Render for GuardStatusItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let mode = self.mode;
        let fill = match mode {
            GuardMode::ReadOnly => Some(
                self.colour
                    .map_or(tokens.colors.warning, |c| badge_colour(c, &tokens)),
            ),
            GuardMode::Guarded => None,
        };
        let text = fill.map_or(tokens.colors.text_muted, |f| label_colour(f, &tokens));
        let label: SharedString = mode.label().into();
        let cluster = self.cluster.clone();
        let run = self.run.clone();
        h_flex()
            .id("status-guard")
            .debug_selector(|| "status-guard".to_owned())
            .gap(u(tokens.spacing.xs))
            .px(u(tokens.spacing.sm))
            .h_full()
            .items_center()
            .cursor_pointer()
            .text_color(text)
            .when_some(fill, |this, fill| this.bg(fill))
            .when(fill.is_none(), |this| {
                this.hover(|this| this.bg(tokens.colors.element_hover))
            })
            .tooltip(move |window, cx| Tooltip::new(mode.tooltip()).build(window, cx))
            .on_click(move |_, window, cx| {
                if let (Some(cluster), Some(run)) = (cluster.clone(), run.as_ref()) {
                    run(mode.toggle(cluster), window, cx);
                }
            })
            .when(mode == GuardMode::ReadOnly, |this| {
                this.child(
                    div()
                        .debug_selector(|| "status-guard-lock".to_owned())
                        .flex_none()
                        .child(Icon::new(IconName::Lock).size(u(px(12.))).color(text)),
                )
            })
            .child(div().text_size(u(tokens.font.small)).child(label))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_click_flips_the_mode() {
        let cluster: ClusterId = "3f2a9c1b7d4e8a60".parse().expect("a valid id");
        assert_eq!(
            GuardMode::Guarded.toggle(cluster.clone()),
            Command::ClusterToggleReadOnly {
                cluster: cluster.clone(),
                read_only: Some(true)
            }
        );
        assert_eq!(
            GuardMode::ReadOnly.toggle(cluster.clone()),
            Command::ClusterToggleReadOnly {
                cluster,
                read_only: Some(false)
            }
        );
    }

    #[test]
    fn each_mode_says_what_it_blocks() {
        assert_eq!(GuardMode::of(true), GuardMode::ReadOnly);
        assert!(GuardMode::ReadOnly.tooltip().contains("blocked"));
        assert!(GuardMode::Guarded.tooltip().contains("confirmation"));
    }
}
