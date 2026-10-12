//! A page with live-looking data and no cluster behind it, for the screenshot tests and anyone
//! who wants to look at the page: [`preview_model`] fills a [`PulsesModel`] the way a busy cluster
//! would, and [`PulsesPreview`] draws it with the page's own layout and tile elements.

use gpui::{Context, IntoElement, ParentElement as _, Render, Styled as _, Window};
use oxikube_app::{CountState, FetchedCount, KindCount};
use oxikube_ui::layout::v_flex;
use oxikube_ui::{ActiveTokens as _, u};

use super::face::{ClusterFigure, Fallback};
use super::grid;
use super::model::PulsesModel;
use super::ring::PULSE_SAMPLES;
use super::specs::TileSource;

/// The seconds the preview's newest sample is at.
pub const PREVIEW_NOW: f64 = 1_791_806_400.0;

fn counted(total: usize, rated: usize, healthy: usize) -> CountState {
    CountState::Counted(KindCount {
        total,
        rated,
        healthy,
    })
}

/// A wave around `base` of `swing`, different for each `seed`, as a count history would wobble.
fn wave(base: f64, swing: f64, seed: usize, i: usize) -> f64 {
    let t = i as f64 / 6.0 + seed as f64;
    (base + swing * (t.sin() + 0.5 * (2.3 * t).sin()))
        .max(0.0)
        .round()
}

/// A model of a busy four-node cluster: every kind counted, a few things not healthy, the
/// ingresses over the watch budget and counted once, and the two gauges with history. The pods
/// tile is selected.
pub fn preview_model() -> PulsesModel {
    let mut model = PulsesModel::new();
    let answers: Vec<CountState> = model
        .tiles()
        .iter()
        .filter(|t| matches!(t.spec.source, TileSource::Kind(_)))
        .map(|t| match t.spec.id {
            "nodes" => counted(4, 4, 4),
            "namespaces" => counted(12, 0, 0),
            "pods" => counted(143, 143, 139),
            "deployments" => counted(31, 31, 29),
            "statefulsets" => counted(4, 4, 4),
            "daemonsets" => counted(6, 6, 6),
            "jobs" => counted(18, 18, 16),
            "cronjobs" => counted(5, 5, 4),
            "persistentvolumes" => counted(22, 0, 0),
            "persistentvolumeclaims" => counted(22, 0, 0),
            "horizontalpodautoscalers" => counted(7, 0, 0),
            "ingresses" => CountState::OverBudget {
                message: "the watch budget of this cluster is used up".into(),
            },
            "networkpolicies" => counted(11, 0, 0),
            _ => counted(46, 0, 0),
        })
        .collect();
    model.set_counts(&answers);
    model.set_fallback(
        "ingresses",
        Fallback::Counted(FetchedCount {
            total: 9,
            exact: true,
        }),
    );
    for (seed, tile) in model.tiles_mut().iter_mut().enumerate() {
        let TileSource::Kind(_) = tile.spec.source else {
            continue;
        };
        let latest = match (&tile.count, &tile.fallback) {
            (CountState::Counted(count), _) => count.total as f64,
            (_, Fallback::Counted(count)) => count.total as f64,
            _ => continue,
        };
        let swing = (latest * 0.08).max(1.0);
        for i in 0..PULSE_SAMPLES {
            let at = PREVIEW_NOW - 5.0 * (PULSE_SAMPLES - 1 - i) as f64;
            // The history ends on the number the tile shows.
            let value = if i + 1 == PULSE_SAMPLES {
                latest
            } else {
                wave(latest, swing, seed, i)
            };
            tile.ring.push(at, value);
        }
    }
    for (id, used, allocatable, base) in [
        ("cluster-cpu", 9_920_000_000_u128, 16_000_000_000_u128, 58.0),
        (
            "cluster-memory",
            50 * 1024 * 1024 * 1024,
            64 * 1024 * 1024 * 1024,
            72.0,
        ),
    ] {
        let percent = used as f64 * 100.0 / allocatable as f64;
        let history: Vec<(f64, f64)> = (0..PULSE_SAMPLES)
            .map(|i| {
                let at = PREVIEW_NOW - 15.0 * (PULSE_SAMPLES - 1 - i) as f64;
                let t = i as f64 / 7.0;
                let v = base + 6.0 * t.sin() + 2.0 * (3.1 * t).sin() + i as f64 * 0.05;
                (at, if i + 1 == PULSE_SAMPLES { percent } else { v })
            })
            .collect();
        model.set_cluster(
            id,
            ClusterFigure::Reading {
                used,
                allocatable,
                percent: Some(percent),
                nodes: 4,
                nodes_reporting: 4,
            },
            history,
        );
    }
    model.select(4);
    model
}

/// The page over a [`PulsesModel`], with nothing behind it: the layout of the real page, not
/// clickable and not keyboard-driven.
pub struct PulsesPreview {
    model: PulsesModel,
}

impl PulsesPreview {
    /// A preview of `model`.
    pub fn new(model: PulsesModel) -> Self {
        Self { model }
    }

    /// The model, for a scenario that feeds it new readings.
    pub fn model_mut(&mut self) -> &mut PulsesModel {
        &mut self.model
    }
}

impl Render for PulsesPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let rows = grid::rows(&mut self.model, &tokens, |_| None);
        v_flex()
            .size_full()
            .overflow_hidden()
            .bg(tokens.colors.background)
            .text_color(tokens.colors.text)
            .p(u(tokens.spacing.xl))
            .gap(u(tokens.spacing.lg))
            .child(grid::header(&tokens))
            .child(v_flex().gap(u(tokens.spacing.lg)).children(rows))
    }
}
