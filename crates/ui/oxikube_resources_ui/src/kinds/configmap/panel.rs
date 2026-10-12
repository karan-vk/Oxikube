//! [`ConfigMapPanel`]: the keys of the ConfigMap in the detail, and the preview of one.

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Styled as _, Subscription, Task, WeakEntity, Window, div, px, uniform_list,
};
use oxikube_domain::view::ConfigMapView;
use oxikube_ui::code_view::{CodeView, Look};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, u};

use super::rows::{binary_row, entry_row, size_text};
use crate::detail::DetailView;
use crate::kinds::key_list;

/// The height of the preview, at 100 % zoom.
const PREVIEW_HEIGHT: f32 = 240.;

/// What the view was made from: another version of the object makes it again.
#[derive(Clone, PartialEq, Eq)]
struct Source {
    version: Option<Arc<str>>,
}

/// The preview of one expanded key.
struct Preview {
    key: Arc<str>,
    view: Entity<CodeView>,
}

/// The ConfigMap panel. See the module docs.
pub struct ConfigMapPanel {
    detail: WeakEntity<DetailView>,
    /// The keys, made off the UI thread.
    view: Option<Arc<ConfigMapView>>,
    source: Option<Source>,
    /// The view being made; a newer object replaces (and so cancels) it.
    building: Option<Task<()>>,
    preview: Option<Preview>,
    _detail: Option<Subscription>,
}

impl ConfigMapPanel {
    /// A panel over `detail`, following it.
    pub fn new(detail: WeakEntity<DetailView>, cx: &mut Context<Self>) -> Self {
        let subscription = detail
            .upgrade()
            .map(|entity| cx.observe(&entity, |this, _, cx| this.refresh(cx)));
        // The object may be known already; the detail is being drawn (it built this panel), so
        // it is read after the draw, not now.
        let panel = cx.entity().downgrade();
        cx.defer(move |cx| {
            panel.update(cx, |this, cx| this.refresh(cx)).ok();
        });
        Self {
            detail,
            view: None,
            source: None,
            building: None,
            preview: None,
            _detail: subscription,
        }
    }

    /// The keys, once made.
    pub fn view(&self) -> Option<&Arc<ConfigMapView>> {
        self.view.as_ref()
    }

    /// The key whose preview is open.
    pub fn expanded(&self) -> Option<&str> {
        self.preview.as_ref().map(|preview| &*preview.key)
    }

    /// The preview's code view, while a key is expanded.
    pub fn preview_view(&self) -> Option<&Entity<CodeView>> {
        self.preview.as_ref().map(|preview| &preview.view)
    }

    /// Makes the keys again when the object has a new version. The object must be complete (the
    /// feed carries metadata only; the detail's full read fills it in).
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let resource = detail
            .read(cx)
            .resource()
            .filter(|resource| !resource.is_partial())
            .cloned();
        let Some(resource) = resource else {
            return;
        };
        let source = Source {
            version: resource.meta.resource_version.clone(),
        };
        if self.source.as_ref() == Some(&source) {
            return;
        }
        self.source = Some(source);
        self.building = Some(cx.spawn(async move |this, cx| {
            let made = cx
                .background_spawn(async move { ConfigMapView::from_resource(&resource) })
                .await;
            this.update(cx, |panel, cx| panel.made(made.ok(), cx)).ok();
        }));
    }

    fn made(&mut self, view: Option<ConfigMapView>, cx: &mut Context<Self>) {
        let Some(view) = view else {
            return;
        };
        // A key that is gone closes its preview; one that changed shows the new text.
        if let Some(open) = self.preview.take() {
            if let Some(entry) = view.entries.iter().find(|e| e.key == open.key) {
                let text: Arc<str> = Arc::from(entry.preview());
                open.view.update(cx, |code, cx| code.set_text(text, cx));
                self.preview = Some(open);
            }
        }
        self.view = Some(Arc::new(view));
        cx.notify();
    }

    /// Expands `key` into the preview, or closes it when it is the one open.
    pub fn toggle(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.expanded() == Some(key) {
            self.preview = None;
            cx.notify();
            return;
        }
        let Some(entry) = self
            .view
            .as_ref()
            .and_then(|view| view.entries.iter().find(|e| &*e.key == key))
        else {
            return;
        };
        // The code view is made and coloured now, when a key is first expanded.
        let look = Look {
            language: entry.language.grammar(),
            line_numbers: true,
            soft_wrap: true,
        };
        let text: Arc<str> = Arc::from(entry.preview());
        let key = entry.key.clone();
        let view = cx.new(|cx| CodeView::new(look, cx));
        view.update(cx, |code, cx| code.set_text(text, cx));
        self.preview = Some(Preview { key, view });
        cx.notify();
    }
}

impl Render for ConfigMapPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let Some(view) = self.view.clone() else {
            return key_list::reading(&tokens, "configmap-panel", "ConfigMap");
        };
        let text_rows = view.entries.len();
        let total = view.len();
        let header = h_flex()
            .px(u(tokens.spacing.lg))
            .pt(u(tokens.spacing.lg))
            .pb(u(tokens.spacing.sm))
            .gap(u(tokens.spacing.md))
            .text_size(u(tokens.font.small))
            .text_color(colors.text_muted)
            .child(
                div()
                    .debug_selector(|| "configmap-count".to_owned())
                    .child(match total {
                        1 => "1 KEY".to_owned(),
                        n => format!("{n} KEYS"),
                    }),
            )
            .when(!view.binary.is_empty(), |row| {
                row.child(format!("{} binary", view.binary.len()))
            })
            .when(view.immutable, |row| row.child("immutable"));
        let body = if total == 0 {
            key_list::empty(&tokens, "configmap-empty", "No data")
        } else {
            div()
                .debug_selector(|| "configmap-keys".to_owned())
                .h(key_list::list_height(total))
                .w_full()
                .child(
                    uniform_list(
                        "configmap-keys-list",
                        total,
                        cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                            let tokens = cx.tokens();
                            let Some(view) = this.view.clone() else {
                                return Vec::new();
                            };
                            range
                                .map(|ix| {
                                    if let Some(entry) = view.entries.get(ix) {
                                        let key = entry.key.clone();
                                        let selected = this.expanded() == Some(&*key);
                                        let on_click = cx
                                            .listener(move |this, _, _, cx| this.toggle(&key, cx));
                                        entry_row(&tokens, ix, entry, selected, on_click)
                                    } else {
                                        let binary = &view.binary[ix - text_rows];
                                        binary_row(&tokens, ix, binary)
                                    }
                                })
                                .collect()
                        }),
                    )
                    .size_full(),
                )
                .into_any_element()
        };
        let preview = self.preview.as_ref().map(|preview| {
            let entry = view.entries.iter().find(|e| e.key == preview.key);
            let (language, size, truncated) = entry.map_or(("text", 0, false), |e| {
                (e.language.label(), e.size, e.truncated())
            });
            v_flex()
                .debug_selector(|| "configmap-preview".to_owned())
                .mt(u(tokens.spacing.sm))
                .border_t_1()
                .border_color(colors.border_variant)
                .child(
                    h_flex()
                        .px(u(tokens.spacing.lg))
                        .py(u(tokens.spacing.sm))
                        .gap(u(tokens.spacing.md))
                        .text_size(u(tokens.font.small))
                        .text_color(colors.text_muted)
                        .child(div().text_color(colors.text).child(preview.key.to_string()))
                        .child(language)
                        .child(size_text(size))
                        .when(truncated, |row| {
                            row.child(format!(
                                "showing the first {}",
                                size_text(oxikube_domain::view::PREVIEW_CAP)
                            ))
                        }),
                )
                .child(
                    div()
                        .debug_selector(|| "configmap-preview-text".to_owned())
                        .h(u(px(PREVIEW_HEIGHT)))
                        .w_full()
                        .child(preview.view.clone()),
                )
        });
        v_flex()
            .id("configmap-panel")
            .debug_selector(|| "configmap-panel".to_owned())
            .child(header)
            .child(body)
            .children(preview)
            .into_any_element()
    }
}
