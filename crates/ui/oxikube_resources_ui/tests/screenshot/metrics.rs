//! The Metrics tab of a detail panel (E13-S08), over a scripted Prometheus:
//!
//! - `detail_metrics_pod_dark`, `detail_metrics_pod_light`: a pod with two containers; CPU and
//!   memory with one line per container, each container's request and the limit as reference
//!   lines, the range selector on 1h, and the network chart below. The clock is pinned and the
//!   time axis is UTC, so the picture does not change with the machine.
//! - `detail_metrics_error_dark`: the same pod when Prometheus cannot be reached: the reason
//!   inline with Retry, and no chart frame.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::BoxStream;
use gpui::{AppContext as _, px, size};
use jiff::Timestamp;
use oxikube_app::metrics_history::HistorySource;
use oxikube_app::store::ResourceStores;
use oxikube_app::{CoreColumns, HistoryAvailability, MetricsHistoryService};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::{MetricsProviderKind, OxiResult, Resource};
use oxikube_ports::{ClockPort, PromqlError, PromqlPort, PromqlSeries, PromqlValue, TimeRange};
use oxikube_resources_ui::detail::{DetailDeps, DetailTab, DetailView, MetricsHistory, Mount};
use oxikube_resources_ui::kinds;
use oxikube_resources_ui::table::store_runtime;
use oxikube_testkit::{FakeClockPort, FakeQueryCataloguePort, screenshot::RgbaImage};
use serde_json::json;

use super::pods::connected;
use super::{Ignore, check, headless};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = 820.0;
const ERROR_HEIGHT: f32 = 320.0;

/// Seconds since the epoch of the pinned "now".
const NOW: &str = "2026-01-10T12:00:00Z";

/// The pod: two containers, so CPU and memory have two lines each.
fn two_container_pod() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "api-0", "namespace": "shop", "resourceVersion": "9",
                     "creationTimestamp": "2026-01-01T00:00:00Z"},
        "spec": {"nodeName": "kind-worker", "containers": [
            {"name": "app", "image": "registry.example/api:2.1"},
            {"name": "proxy", "image": "registry.example/proxy:1.4"}
        ]},
        "status": {"phase": "Running", "podIP": "10.244.1.17",
                   "containerStatuses": [
                       {"name": "app", "ready": true, "restartCount": 0,
                        "state": {"running": {"startedAt": "2026-01-01T00:00:06Z"}}},
                       {"name": "proxy", "ready": true, "restartCount": 0,
                        "state": {"running": {"startedAt": "2026-01-01T00:00:06Z"}}}]}
    }))
    .expect("a pod")
}

struct Ready;

impl HistorySource for Ready {
    fn availability(&self, _: &ClusterId) -> HistoryAvailability {
        HistoryAvailability::Ready {
            provider: MetricsProviderKind::KubePrometheus,
        }
    }

    fn changes(&self, _: &ClusterId) -> BoxStream<'static, ()> {
        Box::pin(futures::stream::pending())
    }
}

/// A Prometheus that answers from the placeholder query text, or refuses.
struct Scripted {
    unreachable: bool,
}

fn line(range: TimeRange, labels: &[(&str, &str)], f: impl Fn(f64) -> f64) -> PromqlSeries {
    let mut points = Vec::new();
    let mut at = range.start;
    let total = (range.end.as_second() - range.start.as_second()) as f64;
    while at <= range.end {
        let x = (at.as_second() - range.start.as_second()) as f64 / total;
        points.push(PromqlValue {
            ts: at,
            value: f(x),
        });
        at += range.step * 8;
    }
    PromqlSeries {
        labels: labels
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect::<BTreeMap<_, _>>(),
        points,
    }
}

fn flat(range: TimeRange, labels: &[(&str, &str)], value: f64) -> PromqlSeries {
    PromqlSeries {
        labels: labels
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        points: vec![PromqlValue {
            ts: range.end,
            value,
        }],
    }
}

#[async_trait]
impl PromqlPort for Scripted {
    async fn is_available(&self, _: &ClusterId) -> OxiResult<bool> {
        Ok(!self.unreachable)
    }

    async fn query_instant(
        &self,
        _: &ClusterId,
        _: &str,
        _: Option<Timestamp>,
    ) -> OxiResult<Vec<PromqlSeries>> {
        Ok(Vec::new())
    }

    async fn query_range(
        &self,
        _: &ClusterId,
        query: &str,
        range: TimeRange,
    ) -> OxiResult<Vec<PromqlSeries>> {
        if self.unreachable {
            return Err(PromqlError::Unreachable(
                "the service proxy refused the connection".into(),
            )
            .into());
        }
        let app = [("container", "app")];
        let proxy = [("container", "proxy")];
        let wave = |phase: f64, base: f64, swing: f64| {
            move |x: f64| base + swing * (0.5 + 0.5 * (x * 9.0 + phase).sin())
        };
        Ok(if query.contains("container:cpu_usage") {
            vec![
                line(range, &app, wave(0.0, 0.18, 0.16)),
                line(range, &proxy, wave(1.5, 0.02, 0.03)),
            ]
        } else if query.contains("container:memory_usage") {
            vec![
                line(range, &app, |x| 190e6 + 90e6 * x),
                line(range, &proxy, wave(0.7, 36e6, 6e6)),
            ]
        } else if query.contains("container:cpu_requests") {
            vec![flat(range, &app, 0.25), flat(range, &proxy, 0.05)]
        } else if query.contains("container:cpu_limits") {
            vec![flat(range, &app, 0.5)]
        } else if query.contains("container:memory_requests") {
            vec![flat(range, &app, 256e6), flat(range, &proxy, 64e6)]
        } else if query.contains("container:memory_limits") {
            vec![flat(range, &app, 384e6)]
        } else if query.contains("network_rx") {
            vec![line(range, &[], wave(0.3, 12e3, 8e3))]
        } else if query.contains("network_tx") {
            vec![line(range, &[], wave(2.1, 6e3, 3e3))]
        } else {
            Vec::new()
        })
    }
}

fn render(light: bool, unreachable: bool, height: f32) -> anyhow::Result<RgbaImage> {
    let (sessions, cluster) = connected([two_container_pod()]);
    let target = ResourceRef::namespaced(cluster, Gvk::new("", "v1", "Pod"), "shop", "api-0");
    let clock = Arc::new(FakeClockPort::default());
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(height)), |_, cx| {
        oxikube_ui::init(cx);
        let tokens = if light {
            oxikube_ui::Tokens::light()
        } else {
            oxikube_ui::Tokens::dark()
        };
        oxikube_ui::set_tokens(cx, tokens);
        oxikube_runtime::init_deterministic(cx);
        cx.set_reduce_motion(true);
        kinds::pods::init(cx);
        MetricsHistory::install(
            MetricsHistoryService::new(
                Arc::new(Ready),
                Arc::new(Scripted { unreachable }),
                Arc::new(FakeQueryCataloguePort::new()),
            ),
            cx,
        );
        let clock: Arc<dyn ClockPort> = clock;
        let deps = DetailDeps {
            sessions,
            stores: Arc::new(ResourceStores::new(store_runtime(clock, cx))),
            columns: Arc::new(CoreColumns::new()),
            dispatcher: Rc::new(Ignore),
            exec: None,
            scale: None,
        };
        cx.new(|cx| DetailView::new(target, deps, Mount::Drawer, cx))
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |view, _, cx| {
        let detail = view.downcast::<DetailView>().expect("the root view");
        let now: Timestamp = NOW.parse().expect("a timestamp");
        detail.update(cx, |detail, cx| {
            detail.pin_now(now, cx);
            detail.set_tab(DetailTab::Metrics, cx);
        });
    })?;
    // The first frame builds the view; the answer lands right after; the next frames draw it.
    for _ in 0..3 {
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    }
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

pub(crate) fn run() -> anyhow::Result<()> {
    check(
        "detail_metrics_pod_dark",
        render(false, false, HEIGHT)?,
        WIDTH,
        HEIGHT,
    )?;
    check(
        "detail_metrics_pod_light",
        render(true, false, HEIGHT)?,
        WIDTH,
        HEIGHT,
    )?;
    check(
        "detail_metrics_error_dark",
        render(false, true, ERROR_HEIGHT)?,
        WIDTH,
        ERROR_HEIGHT,
    )
}
