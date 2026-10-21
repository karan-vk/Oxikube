//! [`HelmPage`]: the Helm page of a cluster tab (E16-S11), opened by the sidebar's Helm entry.
//!
//! It hosts the guidance banner and lists what Helm can do here: only the features the gate
//! allows are drawn; the banner explains the missing ones. The releases table and release detail (E16-S05) mount into the same page and
//! show under the banner; until then the page is the place where a missing or too-old `helm`
//! is explained and fixed.

use std::rc::Rc;

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Window, div, px,
};
use oxikube_domain::helm::HelmFeature;
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, IconName, u};
use oxikube_workspace::{CommandDispatcher, Item, ItemEvent, TabContent};

use crate::banner::{HelmBanner, gated};
use crate::gate::HelmGateState;

/// The deduplication key of the tab: opening the page twice shows the open one.
pub const ITEM_KEY: &str = "cluster-helm";

/// The Helm page. See the [module docs](self).
pub struct HelmPage {
    state: Entity<HelmGateState>,
    banner: Entity<HelmBanner>,
    focus: FocusHandle,
    _follow: gpui::Subscription,
}

impl EventEmitter<ItemEvent> for HelmPage {}

impl Focusable for HelmPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl HelmPage {
    /// The page over `state`, its banner's buttons dispatching through `dispatcher`.
    pub fn new(
        state: Entity<HelmGateState>,
        dispatcher: Rc<dyn CommandDispatcher>,
        cx: &mut Context<Self>,
    ) -> Self {
        let banner = cx.new(|cx| HelmBanner::new(state.clone(), dispatcher, cx));
        let follow = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            banner,
            focus: cx.focus_handle(),
            _follow: follow,
        }
    }

    /// The banner entity (tests read it).
    pub fn banner(&self) -> &Entity<HelmBanner> {
        &self.banner
    }
}

impl Item for HelmPage {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new("Helm").icon(IconName::Box)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(ITEM_KEY.into())
    }
}

/// How a feature reads in the availability list.
fn label(feature: HelmFeature) -> &'static str {
    match feature {
        HelmFeature::Releases => "Releases",
        HelmFeature::History => "History",
        HelmFeature::Values => "Values",
        HelmFeature::Manifest => "Manifest",
        HelmFeature::Notes => "Notes",
        HelmFeature::Resources => "Resources",
        HelmFeature::Install => "Install",
        HelmFeature::Upgrade => "Upgrade",
        HelmFeature::Rollback => "Rollback",
        HelmFeature::Uninstall => "Uninstall",
        HelmFeature::Repos => "Repositories",
        HelmFeature::Charts => "Charts catalog",
    }
}

impl Render for HelmPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let state = self.state.read(cx);
        let version = state
            .capabilities()
            .cli_version
            .filter(|_| state.capabilities().cli_available)
            .map_or_else(|| "no helm binary".to_owned(), |v| format!("helm {v}"));
        // Hidden features are not drawn at all: the banner says why they are missing.
        let rows = HelmFeature::ALL.into_iter().filter_map(|feature| {
            gated(state.gate(), feature, || {
                h_flex()
                    .id(("helm-feature", feature as usize))
                    .debug_selector(move || format!("helm-feature-{}", feature.name()))
                    .gap(u(tokens.spacing.md))
                    .text_size(u(px(13.)))
                    .child(label(feature))
            })
        });
        v_flex()
            .id("helm-page")
            .debug_selector(|| "helm-page".into())
            .size_full()
            .gap(u(tokens.spacing.lg))
            .p(u(tokens.spacing.xxl))
            .bg(colors.background)
            .text_color(colors.text)
            .child(div().text_size(u(px(16.))).child("Helm"))
            .child(
                div()
                    .debug_selector(|| "helm-banner-slot".into())
                    .child(self.banner.clone()),
            )
            .child(
                div()
                    .id("helm-version")
                    .debug_selector(|| "helm-version".into())
                    .text_size(u(px(13.)))
                    .text_color(colors.text_muted)
                    .child(version),
            )
            .child(
                v_flex()
                    .id("helm-features")
                    .gap(u(tokens.spacing.xs))
                    .children(rows),
            )
    }
}
