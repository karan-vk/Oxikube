//! The storage detail panel: the object's storage facts above the generic Overview.

use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AnyView, App, AppContext as _, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    WeakEntity, Window, div, px,
};
use oxikube_app::{PvcUsage as _, PvcUsageFeed, UsageStatus};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Tokens, u};

use super::content::{Block, Field, Link, PanelContent, content_of, usage_block};
use crate::detail::DetailView;
use crate::kinds::storage::actions::is_claim;
use crate::kinds::storage::usage::usage_feed;
use crate::table::ToneColors;

/// How often the claim panel looks for a change of the mounting Pods once they are known.
const USAGE_POLL: Duration = Duration::from_secs(1);

/// How often it looks while they are still being read (the list lands within a moment).
const USAGE_POLL_WARMING: Duration = Duration::from_millis(200);

/// The width the labels share, unscaled pixels (the Overview's metadata rows use the same).
const LABEL_WIDTH: f32 = 150.;

/// The tallest the panel grows before it scrolls, so the Overview below stays reachable.
const MAX_HEIGHT: f32 = 300.;

/// The panel of one storage object. See the [module docs](super).
pub struct StoragePanel {
    detail: WeakEntity<DetailView>,
    /// The object version the content was built from (`uid`, `resourceVersion`).
    built_from: Option<(Option<Arc<str>>, Option<Arc<str>>)>,
    content: PanelContent,
    /// For a claim: the feed that knows which Pods mount it, and the list built from it.
    usage: Option<Arc<PvcUsageFeed>>,
    usage_block: Option<Block>,
    usage_version: u64,
    /// Whether the Pods have not been listed yet (the panel looks again soon).
    warming: bool,
    _observe: Option<Subscription>,
    /// Looks for a change of the mounting Pods; started once the detail is known to be a claim's.
    _poll: Option<Task<()>>,
}

impl StoragePanel {
    /// The panel for `detail`, as a view the detail draws above its Overview.
    pub fn build(detail: WeakEntity<DetailView>, _: &mut Window, cx: &mut App) -> AnyView {
        cx.new(|cx| Self::new(detail, cx)).into()
    }

    fn new(detail: WeakEntity<DetailView>, cx: &mut Context<Self>) -> Self {
        let observe = detail
            .upgrade()
            .map(|view| cx.observe(&view, |this, _, cx| this.refresh(cx)));
        // The factory runs while the detail is being drawn, so the detail cannot be read yet: the
        // first fill happens right after the frame.
        cx.spawn(async move |this, cx| {
            this.update(cx, |this, cx| this.refresh(cx)).ok();
        })
        .detach();
        Self {
            detail,
            built_from: None,
            content: PanelContent::default(),
            usage: None,
            usage_block: None,
            usage_version: u64::MAX,
            warming: true,
            _observe: observe,
            _poll: None,
        }
    }

    /// The content now shown (tests and diagnostics).
    pub fn content(&self) -> &PanelContent {
        &self.content
    }

    /// The "Mounted by" list now shown, for a claim.
    pub fn usage_block(&self) -> Option<&Block> {
        self.usage_block.as_ref()
    }

    /// Rebuilds the content when the object has a new version, and the claim's Pods when the
    /// session gained a store.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let (key, content, claim) = {
            let view = detail.read(cx);
            let Some(resource) = view.resource() else {
                return;
            };
            let key = (
                resource.meta.uid.clone(),
                resource.meta.resource_version.clone(),
            );
            if self.built_from.as_ref() == Some(&key)
                && self.usage.is_some() == is_claim(&resource.kind)
            {
                return;
            }
            let claim = is_claim(&resource.kind).then(|| {
                (
                    view.columns_context(),
                    resource.meta.namespace.clone(),
                    resource.meta.name.clone(),
                )
            });
            (key, content_of(resource), claim)
        };
        self.built_from = Some(key);
        self.content = content.unwrap_or_default();
        if let Some((env, namespace, name)) = claim {
            if self._poll.is_none() {
                self._poll = Some(Self::poll_task(cx));
            }
            if self.usage.is_none() {
                self.usage = usage_feed(&env);
            }
            self.usage_version = u64::MAX;
            self.rebuild_usage(namespace.as_deref().unwrap_or_default(), &name);
        }
        cx.notify();
    }

    /// The claim's namespace and name, from the detail.
    fn claim(&self, cx: &App) -> Option<(Arc<str>, Arc<str>)> {
        let detail = self.detail.upgrade()?;
        let target = detail.read(cx).target();
        Some((target.namespace.clone()?, target.name.clone()))
    }

    fn rebuild_usage(&mut self, namespace: &str, name: &str) {
        let Some(feed) = &self.usage else {
            return;
        };
        let index = feed.index();
        self.warming = index.status() == UsageStatus::Warming;
        self.usage_version = index.version();
        self.usage_block = Some(usage_block(&**index, namespace, name));
    }

    /// The task that looks for a change of the mounting Pods: every moment while they are being
    /// read, then once a second.
    fn poll_task(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let mut wait = USAGE_POLL_WARMING;
            loop {
                cx.background_executor().timer(wait).await;
                let next = this.update(cx, |this, cx| {
                    this.poll_usage(cx);
                    if this.warming {
                        USAGE_POLL_WARMING
                    } else {
                        USAGE_POLL
                    }
                });
                match next {
                    Ok(next) => wait = next,
                    Err(_) => break,
                }
            }
        })
    }

    /// Looks for a change of the mounting Pods; redraws only when there is one.
    fn poll_usage(&mut self, cx: &mut Context<Self>) {
        let is_claim_detail = self
            .detail
            .upgrade()
            .is_some_and(|detail| is_claim(&detail.read(cx).target().gvk));
        if !is_claim_detail {
            return;
        }
        // A session that connected after the panel opened gets its feed now.
        if self.usage.is_none() {
            self.built_from = None;
            self.refresh(cx);
            return;
        }
        let Some(feed) = &self.usage else {
            return;
        };
        if feed.index().version() == self.usage_version {
            return;
        }
        if let Some((namespace, name)) = self.claim(cx) {
            self.rebuild_usage(&namespace, &name);
            cx.notify();
        }
    }

    /// Opens the detail of `link`'s object.
    fn open(&mut self, link: &Link, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let (cluster, dispatcher) = {
            let view = detail.read(cx);
            (view.target().cluster.clone(), view.dispatcher())
        };
        let target = match &link.namespace {
            Some(ns) => {
                ResourceRef::namespaced(cluster, link.gvk.clone(), ns.clone(), link.name.clone())
            }
            None => ResourceRef::cluster_scoped(cluster, link.gvk.clone(), link.name.clone()),
        };
        dispatcher.dispatch(Command::ResourceOpen { target }, cx);
    }

    fn field_row(
        &self,
        tokens: &Tokens,
        tones: &ToneColors,
        index: usize,
        field: &Field,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = tokens.colors;
        let link = field.link.clone();
        let value = div()
            .id(("storage-field-value", index))
            .flex_1()
            .min_w_0()
            .whitespace_normal()
            .text_color(match &link {
                Some(_) => colors.accent,
                None => tones.of(field.tone),
            })
            .when_some(link, |value, link| {
                value
                    .cursor_pointer()
                    .hover(|style| style.underline())
                    .on_click(cx.listener(move |this, _, _, cx| this.open(&link, cx)))
            })
            .child(field.value.clone());
        let label = field.label;
        h_flex()
            .debug_selector(move || format!("storage-field-{label}"))
            .items_start()
            .gap(u(tokens.spacing.md))
            .px(u(tokens.spacing.lg))
            .py(u(px(2.)))
            .child(
                div()
                    .flex_none()
                    .w(u(px(LABEL_WIDTH)))
                    .text_color(colors.text_muted)
                    .child(label),
            )
            .child(value)
            .into_any_element()
    }

    fn block_view(
        &self,
        tokens: &Tokens,
        base: usize,
        block: &Block,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = tokens.colors;
        let title = block.title;
        v_flex()
            .debug_selector(move || format!("storage-block-{title}"))
            .child(
                div()
                    .px(u(tokens.spacing.lg))
                    .pt(u(tokens.spacing.md))
                    .pb(u(tokens.spacing.sm))
                    .text_size(u(tokens.font.small))
                    .font_semibold()
                    .text_color(colors.text_muted)
                    .child(title.to_uppercase()),
            )
            .children(block.lines.iter().enumerate().map(|(i, line)| {
                let id = base + i;
                let link = line.link.clone();
                div()
                    .id(("storage-line", id))
                    .debug_selector(move || format!("storage-line-{title}-{i}"))
                    .px(u(tokens.spacing.lg))
                    .py(u(px(1.)))
                    .text_color(match (&link, block.note) {
                        (Some(_), _) => colors.accent,
                        (None, true) => colors.text_muted,
                        (None, false) => colors.text,
                    })
                    .when_some(link, |row, link| {
                        row.cursor_pointer()
                            .hover(|style| style.underline())
                            .on_click(cx.listener(move |this, _, _, cx| this.open(&link, cx)))
                    })
                    .child(line.text.clone())
            }))
            .into_any_element()
    }
}

impl Render for StoragePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let tones = ToneColors::current(cx);
        let fields: Vec<AnyElement> = self
            .content
            .fields
            .iter()
            .enumerate()
            .map(|(i, field)| self.field_row(&tokens, &tones, i, field, cx))
            .collect();
        let mut base = 0;
        let mut blocks = Vec::new();
        for block in self.content.blocks.iter().chain(self.usage_block.iter()) {
            blocks.push(self.block_view(&tokens, base, block, cx));
            base += block.lines.len();
        }
        v_flex()
            .id("storage-panel")
            .debug_selector(|| "storage-panel".to_owned())
            .w_full()
            .max_h(u(px(MAX_HEIGHT)))
            .overflow_y_scroll()
            .py(u(tokens.spacing.md))
            .border_b_1()
            .border_color(tokens.colors.border)
            .children(fields)
            .children(blocks)
    }
}
