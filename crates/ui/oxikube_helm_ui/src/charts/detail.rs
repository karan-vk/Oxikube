//! The open chart: its version, tab and the texts read for them.

use std::sync::Arc;

use gpui::{AppContext as _, Context, Entity, Task, UniformListScrollHandle};
use oxikube_domain::helm::{ChartPart, ChartSummary};
use oxikube_runtime::spawn_kube;
use oxikube_ui::code_view::{CodeView, Look};
use oxikube_ui::markdown::{MarkdownState, sanitize};

use super::backend::ChartsBackend;
use super::model::{Load, Tab};
use super::view::HelmChartsView;

/// The detail pane's state. Text is read when its tab is first shown and kept while the version
/// stays; changing the version reads again.
pub struct Detail {
    /// The chart version shown (the newest at first).
    pub version: String,
    /// The tab open.
    pub tab: Tab,
    /// How the README is.
    pub readme: Load,
    /// How the default values are.
    pub values: Load,
    /// The README's rendered state, once it arrived. Parsed off the UI thread by the text view.
    pub markdown: Entity<MarkdownState>,
    /// The default values, a virtualised read-only YAML view.
    pub code: Entity<CodeView>,
    /// The sanitised README the text view was given.
    pub readme_text: Option<Arc<str>>,
    /// The README is empty (the chart ships none).
    pub readme_empty: bool,
    /// The values are empty.
    pub values_empty: bool,
    /// The scroll of the versions list.
    pub versions_scroll: UniformListScrollHandle,
    readme_task: Option<Task<()>>,
    values_task: Option<Task<()>>,
}

impl Detail {
    pub(super) fn new(chart: &ChartSummary, cx: &mut Context<HelmChartsView>) -> Self {
        Self {
            version: chart.latest().version.clone(),
            tab: Tab::Readme,
            readme: Load::Idle,
            values: Load::Idle,
            markdown: cx.new(|cx| MarkdownState::markdown("", cx)),
            code: cx.new(|cx| CodeView::new(Look::YAML, cx)),
            readme_text: None,
            readme_empty: false,
            values_empty: false,
            versions_scroll: UniformListScrollHandle::new(),
            readme_task: None,
            values_task: None,
        }
    }

    /// Forgets what was read (the version changed); dropping the tasks cancels reads in flight.
    pub(super) fn reset(&mut self) {
        self.readme = Load::Idle;
        self.values = Load::Idle;
        self.readme_task = None;
        self.values_task = None;
        self.readme_text = None;
    }

    /// Reads `tab`'s text if it was not read yet.
    pub(super) fn load(
        &mut self,
        chart: &ChartSummary,
        tab: Tab,
        backend: &dyn ChartsBackend,
        cx: &mut Context<HelmChartsView>,
    ) {
        let (part, load) = match tab {
            Tab::Readme => (ChartPart::Readme, &mut self.readme),
            Tab::Values => (ChartPart::Values, &mut self.values),
            Tab::Versions => return,
        };
        if *load != Load::Idle {
            return;
        }
        *load = Load::Loading;
        let read = backend.detail(chart.chart_ref(Some(&self.version)), part);
        let task = cx.spawn(async move |this, cx| {
            let text = spawn_kube(cx, read).await;
            let text = text
                .map_err(|e| e.to_string())
                .and_then(|r| r.map_err(|e| e.to_string()));
            // Untrusted Markdown is cleaned off the UI thread; a README can be hundreds of KB.
            let prepared = match (part, text) {
                (ChartPart::Readme, Ok(text)) => {
                    let clean = cx
                        .background_executor()
                        .spawn(async move { Arc::<str>::from(sanitize(&text)) })
                        .await;
                    Ok(clean)
                }
                (_, other) => other,
            };
            this.update(cx, |this, cx| {
                if let Some(detail) = &mut this.detail {
                    detail.arrived(part, prepared, cx);
                }
                cx.notify();
            })
            .ok();
        });
        match part {
            ChartPart::Readme => self.readme_task = Some(task),
            _ => self.values_task = Some(task),
        }
    }

    fn arrived(
        &mut self,
        part: ChartPart,
        text: Result<Arc<str>, String>,
        cx: &mut Context<HelmChartsView>,
    ) {
        match (part, text) {
            (ChartPart::Readme, Ok(text)) => {
                self.readme_empty = text.trim().is_empty();
                self.markdown
                    .update(cx, |state, cx| state.set_text(&text, cx));
                self.readme_text = Some(text);
                self.readme = Load::Ready;
            }
            (_, Ok(text)) => {
                self.values_empty = text.trim().is_empty();
                self.code.update(cx, |code, cx| code.set_text(text, cx));
                self.values = Load::Ready;
            }
            (ChartPart::Readme, Err(why)) => self.readme = Load::Failed(why),
            (_, Err(why)) => self.values = Load::Failed(why),
        }
    }
}
