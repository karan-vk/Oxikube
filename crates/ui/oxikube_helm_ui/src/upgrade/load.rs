//! Loading the tab: whether `helm` is there, the release (or the releases to pick from), the chart
//! sources and the chart's default values at the two versions. Everything runs off the UI thread.

use std::sync::Arc;

use gpui::{Context, Window};
use oxikube_app::{ChartsReadiness, CurrentRelease};
use oxikube_domain::helm::ValuesDocs;
use oxikube_runtime::spawn_kube;

use super::backend::ChartSource;
use super::model::Stage;
use super::view::HelmUpgradeView;

impl HelmUpgradeView {
    /// Looks for `helm` and, when it is there, reads what the form needs; lists the releases when
    /// none was named.
    pub(super) fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ready = self.deps.backend.ready();
        let task = cx.spawn_in(window, async move |this, cx| {
            let readiness = spawn_kube(cx, ready).await;
            let usable = !matches!(readiness, Ok(Ok(ChartsReadiness::NoHelm)));
            this.update_in(cx, |this, window, cx| {
                if usable {
                    this.load_target(window, cx);
                } else {
                    this.stage = Stage::NoHelm;
                }
                cx.notify();
            })
            .ok();
        });
        self.load_tasks.push(task);
    }

    /// Reads the release, or the list of releases to pick from.
    pub(super) fn load_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(release) = self.target.clone() else {
            self.stage = Stage::Choosing;
            let releases = self.deps.backend.releases(self.deps.cluster.clone());
            self.load_tasks.push(cx.spawn(async move |this, cx| {
                let found = spawn_kube(cx, releases).await;
                this.update(cx, |this, cx| {
                    match found
                        .map_err(|e| e.to_string())
                        .and_then(|r| r.map_err(|e| e.to_string()))
                    {
                        Ok(found) => this.releases = Some(found),
                        Err(why) => this.stage = Stage::Failed(why),
                    }
                    cx.notify();
                })
                .ok();
            }));
            return;
        };
        self.stage = Stage::Loading;
        let current = self.deps.backend.current(release);
        self.load_tasks
            .push(cx.spawn_in(window, async move |this, cx| {
                let read = spawn_kube(cx, current).await;
                this.update_in(cx, |this, window, cx| {
                    match read
                        .map_err(|e| e.to_string())
                        .and_then(|r| r.map_err(|e| e.to_string()))
                    {
                        Ok(current) => this.apply_current(current, window, cx),
                        Err(why) => this.stage = Stage::Failed(why),
                    }
                    cx.notify();
                })
                .ok();
            }));
    }

    /// Prefills the editor with the release's current values and looks for the chart's sources.
    fn apply_current(
        &mut self,
        current: CurrentRelease,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text: Arc<str> = Arc::from(current.values.expose());
        self.current_values = text.clone();
        self.set_values(&text, window, cx);
        let name = current.chart.name.clone();
        // Until the repositories answer, the chart is the one the release runs, unqualified.
        self.chart
            .update(cx, |input, cx| input.set_value(name.clone(), window, cx));
        self.current = Some(current);
        self.stage = Stage::Editing;
        self.refresh_values_diff(cx);
        let sources = self.deps.backend.sources(name);
        self.load_tasks
            .push(cx.spawn_in(window, async move |this, cx| {
                if let Ok(Ok(found)) = spawn_kube(cx, sources).await {
                    this.update_in(cx, |this, window, cx| {
                        this.apply_sources(found, window, cx);
                    })
                    .ok();
                }
            }));
    }

    /// Takes the charts the repositories offer under the release's chart name; the one that has
    /// the version the release runs (else the first) is the default source.
    fn apply_sources(
        &mut self,
        sources: Vec<ChartSource>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let running = self.current.as_ref().map(|c| c.chart.version.clone());
        let best = sources
            .iter()
            .find(|s| running.as_ref().is_some_and(|v| s.versions.contains(v)))
            .or(sources.first())
            .cloned();
        self.sources = sources;
        if let Some(best) = best {
            let typed = self.chart.read(cx).value().to_string();
            // What the user already typed wins.
            if self.current.as_ref().is_some_and(|c| c.chart.name == typed) {
                let reference = best.chart.reference();
                self.chart
                    .update(cx, |input, cx| input.set_value(reference, window, cx));
            }
        }
        self.load_defaults(cx);
        cx.notify();
    }

    /// The chart source changed (a candidate, or a reference typed): its defaults are read again.
    pub(super) fn source_changed(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.load_defaults(cx);
        cx.notify();
    }

    /// Reads the chart's default values at the version the release runs and at the target
    /// version (cached by the catalog), for the defaults diff and the editor's docs.
    pub(super) fn load_defaults(&mut self, cx: &mut Context<Self>) {
        self.defaults_tasks.clear();
        self.defaults_problem = None;
        let Some(current) = self.current.as_ref() else {
            return;
        };
        let form = self.form(cx);
        let target = form.chart_ref(&self.sources);
        let mut from = target.clone();
        from.version.clone_from(&current.chart.version);
        for (chart, is_target) in [(from, false), (target, true)] {
            let read = self.deps.backend.defaults(chart);
            let assist = self.assist.clone();
            self.defaults_tasks.push(cx.spawn(async move |this, cx| {
                let result = spawn_kube(cx, read).await;
                let text = match result
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| e.to_string()))
                {
                    Ok(text) => text,
                    Err(why) => {
                        this.update(cx, |this, cx| {
                            this.defaults_problem = Some(why);
                            cx.notify();
                        })
                        .ok();
                        return;
                    }
                };
                if is_target {
                    let parse = text.clone();
                    let docs = cx
                        .background_executor()
                        .spawn(async move { ValuesDocs::from_values_yaml(&parse) })
                        .await;
                    assist.set_docs(Arc::new(docs));
                }
                this.update(cx, |this, cx| {
                    if is_target {
                        this.target_defaults = Some(text);
                    } else {
                        this.current_defaults = Some(text);
                    }
                    this.refresh_defaults_diff(cx);
                })
                .ok();
            }));
        }
    }

    /// Computes the chart-defaults diff (both versions' defaults are known).
    fn refresh_defaults_diff(&mut self, cx: &mut Context<Self>) {
        let (Some(from), Some(to)) = (self.current_defaults.clone(), self.target_defaults.clone())
        else {
            return;
        };
        self.load_tasks.push(cx.spawn(async move |this, cx| {
            let diff = cx
                .background_executor()
                .spawn(async move { super::diffs::defaults_diff(&from, &to) })
                .await;
            this.update(cx, |this, cx| {
                this.defaults_diff = Some(Arc::new(diff));
                cx.notify();
            })
            .ok();
        }));
    }
}
