//! The frame of the hotbar and its tiles.
//!
//! A tile is a rounded square with the cluster's initials, a dot in the cluster's colour at its
//! top-right corner, a state dot at its bottom-right corner (connected, connecting, needing
//! auth, failed; none when the cluster is only a favourite), and a bar on its left edge while
//! its tab is the displayed one. Hovering shows the name and the state. Clicking shows the
//! cluster's tab (`cluster::Select`) or, for a favourite that is not connected, connects it
//! (`cluster::Connect`). Dragging a tile onto another moves it there. Right-clicking opens a
//! menu: show or connect, close the tab, add to or remove from favourites.

use gpui::{
    App, AppContext as _, Context, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _, px, uniform_list,
};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::menu::{ContextMenuExt as _, PopupMenuItem};
use oxikube_ui::tooltip::Tooltip;
use oxikube_ui::{ActiveTokens as _, u};
use oxikube_workspace::cluster_tab::cluster_hsla;

use super::model::HotbarEntry;
use super::view::Hotbar;
use crate::catalog::{Badge, Tone};

/// The strip's width at 100 % zoom.
pub const HOTBAR_WIDTH: f32 = 52.;
/// A tile's edge.
const TILE: f32 = 36.;
/// The height of every slot (tile plus gap), uniform as `uniform_list` needs.
const SLOT: f32 = 44.;
/// The key context of the strip.
pub const HOTBAR_CONTEXT: &str = "Hotbar";

/// What a tile being dragged carries: enough to draw the ghost and to find its cluster.
#[derive(Clone)]
pub(super) struct DraggedCluster {
    pub(super) cluster: ClusterId,
    initials: SharedString,
}

impl Render for DraggedCluster {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        div()
            .size(u(px(TILE)))
            .flex()
            .items_center()
            .justify_center()
            .rounded(u(cx.tokens().radius.md))
            .bg(colors.element_selected)
            .border_1()
            .border_color(colors.border_focused)
            .text_color(colors.text)
            .opacity(0.9)
            .child(self.initials.clone())
    }
}

fn tone_colour(tone: Tone, cx: &App) -> gpui::Hsla {
    let colors = cx.colors();
    match tone {
        Tone::Muted => colors.text_muted,
        Tone::Info => colors.info,
        Tone::Success => colors.success,
        Tone::Warning => colors.warning,
        Tone::Error => colors.error,
    }
}

impl Render for Hotbar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        v_flex()
            .id("hotbar")
            .debug_selector(|| "hotbar".to_owned())
            .key_context(HOTBAR_CONTEXT)
            .track_focus(&self.focus)
            .flex_none()
            .w(u(px(HOTBAR_WIDTH)))
            .h_full()
            .items_center()
            .py(u(tokens.spacing.md))
            .bg(colors.surface)
            .border_r_1()
            .border_color(colors.border_variant)
            .child(
                uniform_list(
                    "hotbar-tiles",
                    self.model.len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        let entries = this.model.entries();
                        range
                            .filter_map(|ix| entries.get(ix).map(|entry| this.tile(ix, entry, cx)))
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.scroll)
                .flex_1()
                .min_h_0()
                .w_full(),
            )
    }
}

impl Hotbar {
    fn tile(&self, ix: usize, entry: &HotbarEntry, cx: &mut Context<Self>) -> gpui::AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let badge = Badge::of(None, &entry.state);
        let state_colour = tone_colour(badge.tone, cx);
        let accent = entry.colour.map_or(colors.accent, cluster_hsla);
        let name = entry.name.clone();
        let cluster = entry.cluster.clone();
        let (connected, favourite, active) = (entry.connected, entry.favourite, entry.active);
        let tooltip_text = if connected {
            format!("{} - {}", entry.name, badge.label)
        } else {
            format!("{} - favourite, not connected", entry.name)
        };
        let ghost = DraggedCluster {
            cluster: cluster.clone(),
            initials: entry.initials.clone().into(),
        };

        let tile = div()
            .id(("hotbar-tile", ix))
            .debug_selector({
                let name = name.clone();
                move || format!("hotbar-tile-{name}")
            })
            .relative()
            .size(u(px(TILE)))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(u(tokens.radius.md))
            .cursor_pointer()
            .bg(if active {
                colors.element_selected
            } else {
                colors.element
            })
            .hover(|style| style.bg(colors.element_hover))
            .text_size(u(tokens.font.body))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(if connected {
                colors.text
            } else {
                colors.text_muted
            })
            .child(entry.initials.clone())
            // The cluster's own colour.
            .child(
                div()
                    .debug_selector({
                        let name = name.clone();
                        move || format!("hotbar-colour-{name}")
                    })
                    .absolute()
                    .top(u(px(3.)))
                    .right(u(px(3.)))
                    .size(u(px(8.)))
                    .rounded_full()
                    .bg(accent),
            )
            // What the session is doing; nothing for a favourite that is not connected.
            .when(connected, |this| {
                this.child(
                    div()
                        .debug_selector({
                            let name = name.clone();
                            move || format!("hotbar-state-{name}")
                        })
                        .absolute()
                        .bottom(u(px(3.)))
                        .right(u(px(3.)))
                        .size(u(px(8.)))
                        .rounded_full()
                        .bg(state_colour)
                        .border_1()
                        .border_color(colors.surface),
                )
            })
            .on_click(cx.listener({
                let cluster = cluster.clone();
                move |this, _, _, cx| this.activate(&cluster, cx)
            }))
            .on_drag(ghost, |ghost, _, _, cx| cx.new(|_| ghost.clone()))
            .drag_over::<DraggedCluster>(move |style, _, _, _| {
                style.bg(colors.element_selected).opacity(0.8)
            })
            .on_drop(cx.listener({
                let cluster = cluster.clone();
                move |this, dragged: &DraggedCluster, _, cx| {
                    this.drop_on(&dragged.cluster, &cluster, cx)
                }
            }))
            .tooltip(move |window, cx| Tooltip::new(tooltip_text.clone()).build(window, cx));

        let menu_host = cx.entity().downgrade();
        let menu_cluster = cluster;
        let menu_name = name.clone();
        let tile = tile.context_menu(move |menu, _, _| {
            let host = menu_host.clone();
            let (cluster, name) = (menu_cluster.clone(), menu_name.clone());
            let show = {
                let (host, cluster) = (host.clone(), cluster.clone());
                PopupMenuItem::new(if connected { "Show" } else { "Connect" }).on_click(
                    move |_, _, cx| {
                        host.update(cx, |this, cx| this.activate(&cluster, cx)).ok();
                    },
                )
            };
            let mut menu = menu.item(show);
            if connected {
                let (host, cluster) = (host.clone(), cluster.clone());
                menu = menu.item(PopupMenuItem::new("Close tab").on_click(move |_, _, cx| {
                    host.update(cx, |this, cx| {
                        this.send(
                            Command::ClusterCloseTab {
                                cluster: cluster.clone(),
                            },
                            cx,
                        )
                    })
                    .ok();
                }));
            }
            menu.separator().item(
                PopupMenuItem::new(if favourite {
                    "Remove from favourites"
                } else {
                    "Add to favourites"
                })
                .on_click(move |_, _, cx| {
                    host.update(cx, |this, cx| {
                        this.toggle_favourite(&cluster, &name, !favourite, cx)
                    })
                    .ok();
                }),
            )
        });

        h_flex()
            .id(("hotbar-slot", ix))
            .debug_selector({
                let name = name.clone();
                move || format!("hotbar-slot-{name}")
            })
            .h(u(px(SLOT)))
            .w_full()
            .flex_none()
            .items_center()
            .justify_center()
            .relative()
            // The displayed cluster's bar, on the strip's left edge.
            .when(active, |this| {
                this.child(
                    div()
                        .debug_selector({
                            let name = name.clone();
                            move || format!("hotbar-active-{name}")
                        })
                        .absolute()
                        .left_0()
                        .top(u(px(8.)))
                        .bottom(u(px(8.)))
                        .w(u(px(3.)))
                        .rounded_r(u(px(2.)))
                        .bg(accent),
                )
            })
            // A drag that starts on the tile must not also be a press on the slot.
            .on_mouse_down(MouseButton::Right, |_, _, _| {})
            .child(tile)
            .into_any_element()
    }
}

impl Hotbar {
    /// What a click on a tile does: show the cluster's tab, or connect a favourite that has none.
    pub fn activate(&mut self, cluster: &ClusterId, cx: &mut Context<Self>) {
        let Some(entry) = self.model.find(cluster) else {
            return;
        };
        let command = if entry.connected {
            Command::ClusterSelect {
                cluster: cluster.clone(),
            }
        } else {
            Command::ClusterConnect {
                cluster: cluster.clone(),
            }
        };
        self.send(command, cx);
    }

    /// Marks or unmarks `cluster` as a favourite (`cluster::ToggleFavourite`). The tile follows
    /// at once; the catalog's state catches up behind it.
    pub fn toggle_favourite(
        &mut self,
        cluster: &ClusterId,
        name: &str,
        favourite: bool,
        cx: &mut Context<Self>,
    ) {
        self.apply_favourite(cluster, Some(name.to_owned()), favourite, cx);
        self.send(
            Command::ClusterToggleFavourite {
                cluster: cluster.clone(),
                favourite: Some(favourite),
            },
            cx,
        );
    }

    /// A tile of `dragged` was dropped on the tile of `target`: it takes that tile's place.
    pub fn drop_on(&mut self, dragged: &ClusterId, target: &ClusterId, cx: &mut Context<Self>) {
        let Some(to) = self
            .model
            .shown_order()
            .iter()
            .position(|cluster| cluster == target)
        else {
            return;
        };
        self.move_to(dragged, to, cx);
    }

    /// Moves `cluster` to tile index `to` and saves the new order.
    pub fn move_to(&mut self, cluster: &ClusterId, to: usize, cx: &mut Context<Self>) {
        if self.model.move_entry(cluster, to) {
            self.save_order(cx);
            cx.notify();
        }
    }
}
