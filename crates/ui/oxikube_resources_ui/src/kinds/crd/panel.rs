//! [`CrdPanel`]: what a CRD's detail shows above the generic Overview (E12-S17).
//!
//! Names (kind, plural, singular, list kind, short names, categories), scope, the conversion
//! strategy, `Established` and `NamesAccepted` in brief (the full conditions table is the
//! Overview's), and each version: served, storage, deprecated (with its warning), schema,
//! subresources, and the printer columns `kubectl get` shows for it, the wide ones marked. A
//! button opens the custom resources, as Enter on the CRD's row does.
//!
//! The panel reads the CRD through its [`DetailView`] and builds its [`PanelModel`] once per
//! change of the object (keyed by name and `resourceVersion`); drawing only reads the model. It
//! is as tall as its content up to a cap, then scrolls, so the Overview below keeps room.

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Entity, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, WeakEntity, Window, div, px,
};
use oxikube_domain::view::{ConditionStatus, CrdSummary};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, Tokens, u};

use super::model::{FlagTone, PanelModel, VersionModel};
use crate::detail::DetailView;
use crate::table::ToneColors;

/// The panel's height before it scrolls, unscaled pixels.
const MAX_HEIGHT: f32 = 320.;
/// The width of the facts' keys, unscaled pixels.
const KEY_WIDTH: f32 = 110.;

/// The definition panel of one CRD's detail. See the module docs.
pub struct CrdPanel {
    detail: WeakEntity<DetailView>,
    model: Option<PanelModel>,
    /// The name and `resourceVersion` the model was built at.
    seen: Option<(Arc<str>, Option<Arc<str>>)>,
    /// How many times the model was built (once per change of the CRD).
    builds: usize,
    _observe: Option<Subscription>,
}

impl CrdPanel {
    /// The panel of the CRD `detail` shows, following it.
    pub fn new(detail: WeakEntity<DetailView>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            detail: detail.clone(),
            model: None,
            seen: None,
            builds: 0,
            _observe: None,
        };
        if let Some(view) = detail.upgrade() {
            this._observe = Some(cx.observe(&view, |this, view, cx| this.sync(&view, cx)));
            // Built while the detail draws (it is being updated): read it right after.
            let panel = cx.weak_entity();
            cx.defer(move |cx| {
                panel.update(cx, |this, cx| this.sync(&view, cx)).ok();
            });
        }
        this
    }

    /// What the panel draws; `None` until the detail knows the object.
    pub fn model(&self) -> Option<&PanelModel> {
        self.model.as_ref()
    }

    /// How many times the model was built.
    pub fn builds(&self) -> usize {
        self.builds
    }

    /// Builds the model again when the detail's object changed, not when the detail redrew for
    /// another reason.
    fn sync(&mut self, view: &Entity<DetailView>, cx: &mut Context<Self>) {
        let Some(resource) = view.read(cx).resource() else {
            return;
        };
        let key = (
            resource.meta.name.clone(),
            resource.meta.resource_version.clone(),
        );
        if self.seen.as_ref() == Some(&key) {
            return;
        }
        self.model = CrdSummary::from_resource(resource)
            .ok()
            .map(|crd| PanelModel::new(&crd));
        self.seen = Some(key);
        self.builds += 1;
        cx.notify();
    }

    /// Opens the table of the custom resources (`crd::OpenResources`), as Enter on the CRD's row.
    pub fn open_custom_resources(&mut self, cx: &mut Context<Self>) {
        if let Some(detail) = self.detail.upgrade() {
            detail.update(cx, |detail, cx| detail.open_custom_resources(cx));
        }
    }
}

impl Render for CrdPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let tones = ToneColors::current(cx);
        let Some(model) = self.model.as_ref() else {
            return div().debug_selector(|| "crd-panel-loading".to_owned());
        };
        let open = Button::new("crd-panel-open")
            .label("Open custom resources")
            .ghost()
            .xsmall()
            .disabled(!model.can_open)
            .on_click(cx.listener(|this, _, _, cx| this.open_custom_resources(cx)));
        let chips = h_flex()
            .items_center()
            .gap(u(tokens.spacing.md))
            .child(condition_chip(
                &tokens,
                &tones,
                "Established",
                model.established,
            ))
            .child(condition_chip(
                &tokens,
                &tones,
                "NamesAccepted",
                model.names_accepted,
            ))
            .child(div().flex_1())
            .child(
                div()
                    .debug_selector(|| "crd-panel-open".to_owned())
                    .child(open),
            );
        div().debug_selector(|| "crd-panel".to_owned()).child(
            v_flex()
                .id("crd-panel-scroll")
                .max_h(u(px(MAX_HEIGHT)))
                .overflow_y_scroll()
                .gap(u(tokens.spacing.sm))
                .px(u(tokens.spacing.lg))
                .py(u(tokens.spacing.md))
                .border_b_1()
                .border_color(tokens.colors.border_variant)
                .child(chips)
                .children(
                    model
                        .facts
                        .iter()
                        .map(|(key, value)| fact(&tokens, key, value)),
                )
                .child(heading(&tokens, "Versions"))
                .children(
                    model
                        .versions
                        .iter()
                        .map(|v| version_block(&tokens, &tones, v)),
                ),
        )
    }
}

fn heading(tokens: &Tokens, text: &'static str) -> AnyElement {
    div()
        .pt(u(tokens.spacing.sm))
        .text_size(u(tokens.font.small))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(tokens.colors.text_muted)
        .child(text)
        .into_any_element()
}

fn fact(tokens: &Tokens, key: &'static str, value: &SharedString) -> AnyElement {
    h_flex()
        .debug_selector(move || format!("crd-panel-fact-{key}"))
        .gap(u(tokens.spacing.md))
        .text_size(u(tokens.font.small))
        .child(
            div()
                .flex_none()
                .w(u(px(KEY_WIDTH)))
                .text_color(tokens.colors.text_muted)
                .child(key),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .whitespace_normal()
                .child(value.clone()),
        )
        .into_any_element()
}

fn condition_chip(
    tokens: &Tokens,
    tones: &ToneColors,
    label: &'static str,
    status: ConditionStatus,
) -> AnyElement {
    let colour = match status {
        ConditionStatus::True => tones.ok,
        ConditionStatus::False => tones.error,
        ConditionStatus::Unknown => tones.warn,
    };
    chip(tokens, colour, label)
        .debug_selector(move || format!("crd-panel-condition-{label}"))
        .child(": ")
        .child(status.as_str())
        .into_any_element()
}

fn chip(tokens: &Tokens, colour: Hsla, text: &'static str) -> gpui::Div {
    div()
        .flex()
        .px(u(px(6.)))
        .rounded(u(tokens.radius.sm))
        .bg(tokens.colors.element)
        .text_size(u(tokens.font.small))
        .text_color(colour)
        .child(text)
}

/// One version: its name and flags, the deprecation warning, and its printer columns.
fn version_block(tokens: &Tokens, tones: &ToneColors, version: &VersionModel) -> AnyElement {
    let name = version.name.clone();
    let tone = |t: FlagTone| match t {
        FlagTone::Neutral => tokens.colors.text_muted,
        FlagTone::Ok => tones.ok,
        FlagTone::Warn => tones.warn,
    };
    v_flex()
        .debug_selector(move || format!("crd-panel-version-{name}"))
        .gap(u(px(2.)))
        .child(
            h_flex()
                .gap(u(tokens.spacing.sm))
                .items_center()
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(version.name.clone()),
                )
                .children(
                    version
                        .flags
                        .iter()
                        .map(|(text, t)| chip(tokens, tone(*t), text)),
                ),
        )
        .when_some(version.warning.clone(), |block, warning| {
            block.child(
                div()
                    .text_size(u(tokens.font.small))
                    .text_color(tones.warn)
                    .whitespace_normal()
                    .child(warning),
            )
        })
        .child(
            div()
                .text_size(u(tokens.font.small))
                .text_color(tokens.colors.text_muted)
                .child(if version.columns.is_empty() {
                    "Printer columns: none (kubectl shows Name and Age)"
                } else {
                    "Printer columns"
                }),
        )
        .children(version.columns.iter().map(|column| {
            let (v, c) = (version.name.clone(), column.name.clone());
            div()
                .debug_selector(move || format!("crd-panel-column-{v}-{c}"))
                .pl(u(tokens.spacing.lg))
                .text_size(u(tokens.font.small))
                .when(column.wide, |row| row.text_color(tokens.colors.text_muted))
                .child(column.text.clone())
        }))
        .into_any_element()
}
