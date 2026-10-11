//! Sparkline states: data, none, one sample, gaps; and that a repaint reuses the geometry.

use super::super::spark_state::REBUILDS;
use super::*;
use crate::chart::Tone;
use crate::chart::gallery::fixtures::{wave, with_gaps};
use gpui::{Context, IntoElement, Render, TestAppContext, VisualTestContext, Window, div, px};

struct Row {
    points: Arc<[Point]>,
    width: f32,
}

impl Render for Row {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            div().w(px(self.width)).child(
                Sparkline::new("spark", self.points.clone())
                    .color(SeriesColor::Tone(Tone::Good))
                    .filled(true)
                    .last_marker(true),
            ),
        )
    }
}

fn mount(
    cx: &mut TestAppContext,
    points: Arc<[Point]>,
) -> (gpui::Entity<Row>, &mut VisualTestContext) {
    cx.update(crate::init);
    REBUILDS.with(|n| n.set(0));
    let (row, vcx) = cx.add_window_view(move |_, _| Row {
        points,
        width: 120.0,
    });
    vcx.run_until_parked();
    (row, vcx)
}

fn rebuilds() -> usize {
    REBUILDS.with(std::cell::Cell::get)
}

fn series(n: usize) -> Arc<[Point]> {
    wave("s", 10_000.0, n, 15.0, 5.0, 2.0, 20.0, 1).points
}

fn quads(cx: &mut VisualTestContext) -> usize {
    cx.update(|window, _| window.painted_quads().len())
}

#[gpui::test]
fn a_repaint_reuses_the_geometry(cx: &mut TestAppContext) {
    let points = series(120);
    let (row, cx) = mount(cx, points.clone());
    assert_eq!(rebuilds(), 1);
    for _ in 0..5 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
    assert_eq!(rebuilds(), 1, "five frames, one build");

    // The same Arc again (the parent re-rendered): still cached.
    row.update(cx, |row, cx| {
        row.points = points.clone();
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(rebuilds(), 1);

    // New data, a different width: one build each.
    row.update(cx, |row, cx| {
        row.points = series(121);
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(rebuilds(), 2);
    row.update(cx, |row, cx| {
        row.width = 80.0;
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(rebuilds(), 3);
}

#[gpui::test]
fn no_data_is_a_dashed_baseline_and_data_is_a_path(cx: &mut TestAppContext) {
    let (_, data) = mount(cx, series(60));
    // A path and the last-value dot; the dot is the only quad.
    let with_data = quads(data);
    let empty: Arc<[Point]> = Vec::<Point>::new().into();
    let (_, none) = mount(cx, empty);
    let dashes = quads(none);
    assert!(dashes > 3, "dashes of the empty baseline: {dashes}");
    assert!(with_data <= 2, "{with_data} quads for a drawn line");
}

#[gpui::test]
fn only_gaps_is_no_data(cx: &mut TestAppContext) {
    let gaps: Arc<[Point]> = vec![Point::gap(1.0), Point::gap(2.0)].into();
    let (_, cx) = mount(cx, gaps);
    assert!(quads(cx) > 3);
}

#[gpui::test]
fn one_sample_and_gapped_series_draw(cx: &mut TestAppContext) {
    let (_, one) = mount(cx, vec![Point::new(5.0, 3.0)].into());
    assert_eq!(rebuilds(), 1);
    let dots = quads(one);
    assert!(dots >= 1, "the lone sample is a dot");

    let gapped = with_gaps(wave("g", 10_000.0, 120, 15.0, 5.0, 2.0, 20.0, 2), 30).points;
    let (_, _) = mount(cx, gapped);
    assert_eq!(rebuilds(), 1);
}

#[gpui::test]
fn the_state_keeps_its_arc_alive(cx: &mut TestAppContext) {
    // The cache compares by pointer; holding the Arc means a freed allocation cannot be mistaken
    // for the same data.
    let points = series(30);
    let weak = Arc::downgrade(&points);
    let (row, cx) = mount(cx, points);
    row.update(cx, |row, cx| {
        row.points = series(31);
        cx.notify();
    });
    cx.run_until_parked();
    assert!(weak.upgrade().is_none(), "replaced data is released");
}
