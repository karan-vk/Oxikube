//! `#[gpui::test]`s for each chart state: normal, empty ("No data" with a reason), single sample,
//! gaps, long series, hover tooltip on and off, legend toggling, caching and theme changes.

use super::gallery::fixtures::{wave, with_gaps};
use super::*;
use crate::tokens::{ActiveTokens as _, Tokens};
use gpui::{
    AppContext as _, Bounds, Context, Entity, IntoElement, Modifiers, ParentElement as _, Render,
    Styled as _, TestAppContext, VisualTestContext, Window, div, point, px, size,
};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

/// 2026-10-12 00:00 UTC.
const NOW: f64 = 1_791_763_200.0;
const WIDTH: f32 = 640.0;
const HEIGHT: f32 = 240.0;

struct Host {
    chart: Entity<Chart>,
    width: f32,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(cx.colors().background).child(
            div()
                .w(px(self.width))
                .h(px(HEIGHT))
                .child(self.chart.clone()),
        )
    }
}

fn mount(
    cx: &mut TestAppContext,
    chart: impl FnOnce(&mut Context<Chart>) -> Chart,
) -> (Entity<Chart>, &mut VisualTestContext) {
    let (chart, _, vcx) = mount_host(cx, chart);
    (chart, vcx)
}

fn mount_host(
    cx: &mut TestAppContext,
    chart: impl FnOnce(&mut Context<Chart>) -> Chart,
) -> (Entity<Chart>, Entity<Host>, &mut VisualTestContext) {
    cx.update(crate::init);
    let chart = cx.new(chart);
    let shown = chart.clone();
    let (host, vcx) = cx.add_window_view(move |_, _| Host {
        chart: shown,
        width: WIDTH,
    });
    vcx.run_until_parked();
    (chart, host, vcx)
}

fn minutes(name: &str, count: usize, seed: u64) -> Series {
    wave(name, NOW, count, 15.0, 5.0, 2.0, 20.0, seed)
}

fn axis() -> TimeAxis {
    TimeAxis::utc(NOW - 900.0, NOW)
}

fn feed(chart: &Entity<Chart>, cx: &mut VisualTestContext, series: Vec<Series>) {
    chart.update(cx, |chart, cx| chart.set_series(series, axis(), cx));
    cx.run_until_parked();
}

fn geometry(chart: &Entity<Chart>, cx: &VisualTestContext) -> (usize, Vec<usize>, Vec<usize>) {
    let chart = chart.read_with(cx, |c, _| c.cache.clone());
    let cache = chart.borrow();
    let layout = cache.layout.as_ref().expect("a layout");
    (
        layout.series.len(),
        layout.series.iter().map(|s| s.runs.len()).collect(),
        layout.series.iter().map(|s| s.points.len()).collect(),
    )
}

fn notifications(chart: &Entity<Chart>, cx: &mut VisualTestContext) -> Rc<Cell<usize>> {
    let count = Rc::new(Cell::new(0));
    let seen = count.clone();
    cx.update(|_, cx| {
        cx.observe(chart, move |_, _| seen.set(seen.get() + 1))
            .detach();
    });
    count
}

#[gpui::test]
fn data_draws_a_plot_inside_the_canvas(cx: &mut TestAppContext) {
    let (chart, cx) = mount(cx, |_| Chart::line("t"));
    feed(&chart, cx, vec![minutes("a", 60, 1), minutes("b", 60, 2)]);
    let plot = chart.read_with(cx, |c, _| c.plot_bounds()).expect("a plot");
    let canvas = Bounds::new(point(px(0.), px(0.)), size(px(WIDTH), px(HEIGHT)));
    assert!(
        plot.size.width > px(400.) && plot.size.height > px(120.),
        "{plot:?}"
    );
    assert!(
        canvas.contains(&plot.origin)
            && plot.right() <= canvas.right()
            && plot.bottom() < canvas.bottom()
    );
    // Room for the value labels on the left and the time labels below.
    assert!(plot.origin.x > px(8.) && canvas.bottom() - plot.bottom() > px(12.));
    assert_eq!(chart.read_with(cx, |c, _| c.state()), ChartState::Plot);
    let (series, runs, points) = geometry(&chart, cx);
    assert_eq!((series, runs), (2, vec![1, 1]));
    assert!(points.iter().all(|n| *n == 60), "{points:?}");
}

#[gpui::test]
fn projected_points_stay_inside_the_plot(cx: &mut TestAppContext) {
    let (chart, cx) = mount(cx, |_| Chart::area("t"));
    feed(&chart, cx, vec![minutes("a", 60, 1)]);
    let cache = chart.read_with(cx, |c, _| c.cache.clone());
    let cache = cache.borrow();
    let layout = cache.layout.as_ref().unwrap();
    let (left, right) = (layout.plot.origin.x, layout.plot.right());
    let (top, bottom) = (layout.plot.origin.y, layout.plot.bottom());
    for p in &layout.series[0].points {
        assert!(p.x >= left - px(0.5) && p.x <= right + px(0.5), "x {p:?}");
        assert!(p.y >= top - px(0.5) && p.y <= bottom + px(0.5), "y {p:?}");
    }
}

#[gpui::test]
fn no_data_carries_its_reason(cx: &mut TestAppContext) {
    let (chart, cx) = mount(cx, |_| Chart::line("t"));
    // Nothing fed yet: an explicit empty state, no reason.
    assert_eq!(
        chart.read_with(cx, |c, _| c.state()),
        ChartState::NoData(None)
    );
    assert!(chart.read_with(cx, |c, _| c.plot_bounds()).is_none());

    chart.update(cx, |c, cx| {
        c.set_unavailable(Some("No Prometheus endpoint".into()), cx)
    });
    cx.run_until_parked();
    let reason = chart.read_with(cx, |c, _| c.state());
    assert_eq!(
        reason,
        ChartState::NoData(Some("No Prometheus endpoint".into()))
    );

    // Data arrives but the chart is still marked unavailable: the reason wins.
    feed(&chart, cx, vec![minutes("a", 60, 1)]);
    assert_ne!(chart.read_with(cx, |c, _| c.state()), ChartState::Plot);
    chart.update(cx, |c, cx| c.set_unavailable(None, cx));
    cx.run_until_parked();
    assert_eq!(chart.read_with(cx, |c, _| c.state()), ChartState::Plot);
    assert!(chart.read_with(cx, |c, _| c.plot_bounds()).is_some());
}

#[gpui::test]
fn series_without_readings_in_range_is_no_data(cx: &mut TestAppContext) {
    let (chart, cx) = mount(cx, |_| Chart::line("t"));
    let all_gaps = Series::new("a", vec![Point::gap(NOW - 60.0), Point::gap(NOW - 30.0)]);
    feed(&chart, cx, vec![all_gaps]);
    assert_ne!(
        chart.read_with(cx, |c, _| c.state()),
        ChartState::Plot,
        "NaN is not a reading"
    );
    let old = wave("old", NOW - 7_200.0, 20, 15.0, 1.0, 0.5, 5.0, 3);
    feed(&chart, cx, vec![old]);
    assert_ne!(
        chart.read_with(cx, |c, _| c.state()),
        ChartState::Plot,
        "outside the axis"
    );
}

#[gpui::test]
fn a_single_sample_is_a_lone_run(cx: &mut TestAppContext) {
    let (chart, cx) = mount(cx, |_| Chart::line("t").legend(false));
    feed(
        &chart,
        cx,
        vec![Series::new("only", vec![Point::new(NOW - 300.0, 2.0)])],
    );
    assert_eq!(chart.read_with(cx, |c, _| c.state()), ChartState::Plot);
    let (series, runs, points) = geometry(&chart, cx);
    assert_eq!((series, runs, points), (1, vec![1], vec![1]));
}

#[gpui::test]
fn gaps_break_the_line_instead_of_dropping_to_zero(cx: &mut TestAppContext) {
    let (chart, cx) = mount(cx, |_| Chart::line("t"));
    feed(&chart, cx, vec![with_gaps(minutes("a", 60, 1), 20)]);
    let (_, runs, points) = geometry(&chart, cx);
    assert_eq!(
        runs,
        vec![3],
        "gaps at 20-22 and 40-42 split the series in three"
    );
    assert_eq!(points[0], 60 - 6, "the gap samples are not drawn");
}

#[gpui::test]
fn a_long_series_is_decimated_to_the_plot_width(cx: &mut TestAppContext) {
    let (chart, cx) = mount(cx, |_| Chart::area("t").format(ValueFormat::Bytes));
    let day = wave("mem", NOW, 5_760, 15.0, 1.0e9, 4.0e8, 900.0, 4);
    chart.update(cx, |c, cx| {
        c.set_series(vec![day], TimeAxis::utc(NOW - 86_400.0, NOW), cx)
    });
    cx.run_until_parked();
    let (_, _, points) = geometry(&chart, cx);
    let plot_w = chart
        .read_with(cx, |c, _| c.plot_bounds())
        .unwrap()
        .size
        .width
        .as_f32();
    assert!(
        points[0] <= 4 * plot_w.ceil() as usize,
        "{} points for {plot_w}px",
        points[0]
    );
    assert!(points[0] < 5_760 / 2);
}

#[gpui::test]
fn repainting_reuses_the_geometry_until_something_changes(cx: &mut TestAppContext) {
    let (chart, host, cx) = mount_host(cx, |_| Chart::line("t"));
    let a = minutes("a", 60, 1);
    feed(&chart, cx, vec![a.clone()]);
    let builds = chart.read_with(cx, |c, _| c.geometry_builds());
    assert_eq!(builds, 1);

    // Frames without changes, and the same samples fed again (a poll that found nothing new).
    for _ in 0..3 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
    let count = notifications(&chart, cx);
    feed(&chart, cx, vec![a]);
    assert_eq!(count.get(), 0, "the same samples are not a change");
    assert_eq!(chart.read_with(cx, |c, _| c.geometry_builds()), builds);

    // New samples rebuild once.
    feed(&chart, cx, vec![minutes("a", 61, 1)]);
    assert_eq!(chart.read_with(cx, |c, _| c.geometry_builds()), builds + 1);
    assert_eq!(count.get(), 1);

    // A resize rebuilds once.
    host.update(cx, |host, cx| {
        host.width = 500.0;
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(chart.read_with(cx, |c, _| c.geometry_builds()), builds + 2);
}

#[gpui::test]
fn a_theme_change_repaints_with_the_new_palette(cx: &mut TestAppContext) {
    let (chart, cx) = mount(cx, |_| Chart::line("t"));
    feed(&chart, cx, vec![minutes("a", 60, 1)]);
    let before = chart.read_with(cx, |c, _| c.geometry_builds());
    let dark = chart.read_with(cx, |c, _| {
        c.cache.borrow().layout.as_ref().unwrap().series[0].color
    });
    cx.update(|_, cx| {
        let flipped = if cx.tokens().appearance.is_dark() {
            Tokens::light()
        } else {
            Tokens::dark()
        };
        crate::set_tokens(cx, flipped);
    });
    cx.run_until_parked();
    assert_eq!(chart.read_with(cx, |c, _| c.geometry_builds()), before + 1);
    let light = chart.read_with(cx, |c, _| {
        c.cache.borrow().layout.as_ref().unwrap().series[0].color
    });
    assert_ne!(dark, light);
}

fn hover_at(cx: &mut VisualTestContext, chart: &Entity<Chart>, t: f64, dx: f32) {
    let x = chart
        .read_with(cx, |c, _| c.x_of_time(t))
        .expect("x of time");
    let plot = chart.read_with(cx, |c, _| c.plot_bounds()).unwrap();
    cx.simulate_mouse_move(
        point(x + px(dx), plot.origin.y + plot.size.height / 2.),
        None,
        Modifiers::none(),
    );
    cx.run_until_parked();
}

#[gpui::test]
fn hover_shows_the_sample_under_the_cursor_and_leaving_hides_it(cx: &mut TestAppContext) {
    let (chart, cx) = mount(cx, |_| Chart::line("t").format(ValueFormat::Number));
    let (a, b) = (minutes("api", 60, 1), minutes("db", 60, 2));
    let t = a.points[30].t;
    let expected = (a.points[30].v, b.points[30].v);
    feed(&chart, cx, vec![a, b]);
    assert!(chart.read_with(cx, |c, _| c.hovered_time()).is_none());

    hover_at(cx, &chart, t, 1.0);
    assert_eq!(chart.read_with(cx, |c, _| c.hovered_time()), Some(t));
    let palette = cx.update(|_, cx| ChartPalette::from_tokens(&cx.tokens()));
    let rows = chart.read_with(cx, |c, _| c.hover_rows(&palette));
    let names: Vec<_> = rows.iter().map(|r| r.name.to_string()).collect();
    assert_eq!(names, ["api", "db"]);
    assert_eq!(rows[0].value, ValueFormat::Number.format(expected.0));
    assert_eq!(rows[1].value, ValueFormat::Number.format(expected.1));
    assert_ne!(rows[0].color, rows[1].color);
    let plot = chart.read_with(cx, |c, _| c.plot_bounds()).unwrap();
    assert!(
        rows.iter()
            .all(|r| r.at.y >= px(0.) && r.at.y <= plot.bottom())
    );

    // Leaving the chart hides it.
    cx.simulate_mouse_move(
        point(px(WIDTH + 200.), px(HEIGHT + 100.)),
        None,
        Modifiers::none(),
    );
    cx.run_until_parked();
    assert!(chart.read_with(cx, |c, _| c.hovered_time()).is_none());
    assert!(
        chart
            .read_with(cx, |c, _| c.hover_rows(&palette))
            .is_empty()
    );
}

#[gpui::test]
fn hover_notifies_only_when_the_sample_changes(cx: &mut TestAppContext) {
    let (chart, cx) = mount(cx, |_| Chart::line("t"));
    let a = minutes("a", 60, 1);
    let (t0, t1) = (a.points[30].t, a.points[31].t);
    feed(&chart, cx, vec![a]);
    let count = notifications(&chart, cx);

    hover_at(cx, &chart, t0, 0.0);
    let entered = count.get();
    assert!(entered >= 1);
    // Wiggling within the sample's reach changes nothing.
    for dx in [-2.0, -1.0, 1.0, 2.0, 0.5] {
        hover_at(cx, &chart, t0, dx);
    }
    assert_eq!(count.get(), entered, "same sample, no notify");
    assert_eq!(chart.read_with(cx, |c, _| c.hovered_time()), Some(t0));
    // The next sample is a change.
    hover_at(cx, &chart, t1, 0.0);
    assert_eq!(count.get(), entered + 1);
    assert_eq!(chart.read_with(cx, |c, _| c.hovered_time()), Some(t1));
}

#[gpui::test]
fn no_tooltip_where_the_data_has_not_reached(cx: &mut TestAppContext) {
    let (chart, cx) = mount(cx, |_| Chart::line("t"));
    // Ten minutes of data on a 15 minute axis: the right third is empty.
    let a = wave("a", NOW - 450.0, 30, 15.0, 5.0, 1.0, 10.0, 1);
    feed(&chart, cx, vec![a]);
    hover_at(cx, &chart, NOW - 30.0, 0.0);
    assert!(chart.read_with(cx, |c, _| c.hovered_time()).is_none());
    hover_at(cx, &chart, NOW - 600.0, 0.0);
    assert!(chart.read_with(cx, |c, _| c.hovered_time()).is_some());
}

fn click_legend(cx: &mut VisualTestContext, index: usize) {
    let selector = ["chart-legend-0", "chart-legend-1", "chart-legend-2"][index];
    let bounds = cx.debug_bounds(selector).expect("legend entry");
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn the_legend_switches_series_off_and_on(cx: &mut TestAppContext) {
    let (chart, cx) = mount(cx, |_| Chart::line("t"));
    feed(
        &chart,
        cx,
        vec![
            minutes("api", 60, 1),
            minutes("db", 60, 2),
            minutes("cache", 60, 3),
        ],
    );
    assert_eq!(geometry(&chart, cx).0, 3);

    click_legend(cx, 1);
    assert!(chart.read_with(cx, |c, _| c.is_series_hidden(&"db".into())));
    assert_eq!(
        geometry(&chart, cx).0,
        2,
        "the hidden series leaves the plot"
    );
    let colors: Vec<_> = chart.read_with(cx, |c, _| {
        c.cache
            .borrow()
            .layout
            .as_ref()
            .unwrap()
            .series
            .iter()
            .map(|s| s.color)
            .collect()
    });
    let palette = cx.update(|_, cx| ChartPalette::from_tokens(&cx.tokens()));
    assert_eq!(
        colors,
        [palette.series[0], palette.series[2]],
        "the others keep their colours"
    );

    click_legend(cx, 1);
    assert!(!chart.read_with(cx, |c, _| c.is_series_hidden(&"db".into())));
    assert_eq!(geometry(&chart, cx).0, 3);
}

#[gpui::test]
fn hiding_a_series_rescales_the_axis_and_drops_it_from_the_tooltip(cx: &mut TestAppContext) {
    let (chart, cx) = mount(cx, |_| Chart::line("t"));
    let big = wave("big", NOW, 60, 15.0, 90.0, 5.0, 20.0, 1);
    let small = wave("small", NOW, 60, 15.0, 4.0, 1.0, 20.0, 2);
    let t = small.points[30].t;
    feed(&chart, cx, vec![big, small]);
    let y_before = chart.read_with(cx, |c, _| {
        c.cache.borrow().layout.as_ref().unwrap().series[1].points[30].y
    });
    chart.update(cx, |c, cx| c.toggle_series(&"big".into(), cx));
    cx.run_until_parked();
    let y_after = chart.read_with(cx, |c, _| {
        c.cache.borrow().layout.as_ref().unwrap().series[0].points[30].y
    });
    assert!(
        y_after < y_before,
        "the small series now uses the height: {y_after:?} vs {y_before:?}"
    );
    hover_at(cx, &chart, t, 0.0);
    let palette = cx.update(|_, cx| ChartPalette::from_tokens(&cx.tokens()));
    let names: Vec<_> = chart
        .read_with(cx, |c, _| c.hover_rows(&palette))
        .iter()
        .map(|r| r.name.to_string())
        .collect();
    assert_eq!(names, ["small"]);
}

#[gpui::test]
fn bars_paint_a_quad_each(cx: &mut TestAppContext) {
    let (empty, vcx) = mount(cx, |_| Chart::bar("t").legend(false));
    let baseline = vcx.update(|window, _| window.painted_quads().len());
    drop(empty);
    let (chart, cx) = mount(cx, |_| Chart::bar("t").legend(false));
    feed(&chart, cx, vec![minutes("a", 40, 1)]);
    let drawn = cx.update(|window, _| window.painted_quads().len());
    assert!(
        drawn >= baseline + 40,
        "{drawn} quads for 40 bars over a baseline of {baseline}"
    );
    let bars = chart.read_with(cx, |c, _| {
        c.cache.borrow().layout.as_ref().unwrap().series[0]
            .bars
            .len()
    });
    assert_eq!(bars, 40);
}

#[gpui::test]
fn thresholds_and_bands_are_drawn_only_inside_the_value_range(cx: &mut TestAppContext) {
    let (chart, cx) = mount(cx, |_| {
        Chart::line("t")
            .y_range(YRange::Fixed(0.0, 10.0))
            .thresholds(vec![
                Threshold::new(4.0, "request"),
                Threshold::new(50.0, "off scale"),
            ])
            .bands(vec![
                Band::new(8.0, 10.0, Tone::Critical),
                Band::new(20.0, 30.0, Tone::Warn),
            ])
    });
    feed(&chart, cx, vec![minutes("a", 60, 1)]);
    let (thresholds, bands) = chart.read_with(cx, |c, _| {
        let cache = c.cache.borrow();
        let layout = cache.layout.as_ref().unwrap();
        (layout.thresholds.len(), layout.bands.len())
    });
    assert_eq!((thresholds, bands), (1, 1));
}

#[gpui::test]
fn thresholds_can_be_replaced_on_a_chart_on_screen(cx: &mut TestAppContext) {
    let (chart, cx) = mount(cx, |_| Chart::line("t").y_range(YRange::Fixed(0.0, 10.0)));
    feed(&chart, cx, vec![minutes("a", 60, 1)]);
    let count = |chart: &Entity<Chart>, cx: &mut VisualTestContext| {
        chart.read_with(cx, |c, _| {
            c.cache
                .borrow()
                .layout
                .as_ref()
                .map_or(0, |layout| layout.thresholds.len())
        })
    };
    assert_eq!(count(&chart, cx), 0);
    let lines = vec![
        Threshold::new(2.0, "requests"),
        Threshold::new(6.0, "limits"),
    ];
    chart.update(cx, |c, cx| c.set_thresholds(lines.clone(), cx));
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert_eq!(count(&chart, cx), 2);
    // The same lines again change nothing (no new geometry).
    let builds = chart.read_with(cx, |c, _| c.geometry_builds());
    chart.update(cx, |c, cx| c.set_thresholds(lines, cx));
    cx.run_until_parked();
    assert_eq!(chart.read_with(cx, |c, _| c.geometry_builds()), builds);
}

#[gpui::test]
fn a_tiny_canvas_draws_nothing_and_does_not_panic(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let chart = cx.new(|_| Chart::line("t"));
    let host = chart.clone();
    let (_, vcx) = cx.add_window_view(move |_, _| TinyHost { chart: host });
    chart.update(vcx, |c, cx| {
        c.set_series(vec![minutes("a", 10, 1)], axis(), cx)
    });
    vcx.run_until_parked();
    assert!(chart.read_with(vcx, |c, _| c.plot_bounds()).is_none());
}

struct TinyHost {
    chart: Entity<Chart>,
}

impl Render for TinyHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(div().w(px(20.)).h(px(12.)).child(self.chart.clone()))
    }
}

#[gpui::test]
fn the_gallery_renders_every_state(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let (gallery, vcx) = cx.add_window_view(|_, cx| gallery::ChartGallery::new(NOW, cx));
    vcx.run_until_parked();
    let states: Vec<_> = gallery.read_with(vcx, |g, cx| {
        g.charts()
            .map(|c| c.read(cx).state() != ChartState::Plot)
            .collect()
    });
    assert_eq!(states, [false, false, false, false, false, true, true]);
}

#[gpui::test]
fn a_chart_keeps_the_callers_sample_allocation(cx: &mut TestAppContext) {
    let (chart, cx) = mount(cx, |_| Chart::line("t"));
    let series = minutes("a", 60, 1);
    let points = series.points.clone();
    feed(&chart, cx, vec![series]);
    let stored = chart.read_with(cx, |c, _| c.spec.series[0].points.clone());
    assert!(
        Arc::ptr_eq(&stored, &points),
        "set_series copied the samples"
    );
    // Repainting does not copy them either.
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let stored = chart.read_with(cx, |c, _| c.spec.series[0].points.clone());
    assert!(Arc::ptr_eq(&stored, &points));
}

#[gpui::test]
fn a_data_tick_keeps_the_tooltip_while_its_sample_exists(cx: &mut TestAppContext) {
    let (chart, cx) = mount(cx, |_| Chart::line("t"));
    let a = minutes("a", 60, 1);
    let t = a.points[30].t;
    feed(&chart, cx, vec![a.clone()]);
    hover_at(cx, &chart, t, 0.0);
    assert_eq!(chart.read_with(cx, |c, _| c.hovered_time()), Some(t));

    // The next poll: one more sample at the end, the window slides a little.
    let mut next: Vec<Point> = a.points.to_vec();
    next.push(Point::new(NOW + 15.0, 5.0));
    let next = Series::new("a", next);
    chart.update(cx, |c, cx| {
        c.set_series(vec![next], TimeAxis::utc(NOW - 885.0, NOW + 15.0), cx)
    });
    cx.run_until_parked();
    assert_eq!(
        chart.read_with(cx, |c, _| c.hovered_time()),
        Some(t),
        "the tooltip survives a tick under a still cursor"
    );

    // The overlay was placed from the layout of the new data, not the previous frame's.
    assert!(
        overlay_is_current(&chart, cx),
        "the tooltip follows the new geometry"
    );

    // Once the sample has left the data, the tooltip goes.
    let rest = Series::new("a", a.points[40..].to_vec());
    chart.update(cx, |c, cx| c.set_series(vec![rest], axis(), cx));
    cx.run_until_parked();
    assert!(chart.read_with(cx, |c, _| c.hovered_time()).is_none());
}

fn overlay_is_current(chart: &Entity<Chart>, cx: &VisualTestContext) -> bool {
    chart.read_with(cx, |c, _| {
        let cache = c.cache.borrow();
        cache.overlay_build == Some(cache.build)
    })
}

#[gpui::test]
fn a_resize_under_the_cursor_repositions_the_tooltip(cx: &mut TestAppContext) {
    let (chart, host, cx) = mount_host(cx, |_| Chart::line("t"));
    let a = minutes("a", 60, 1);
    let t = a.points[30].t;
    feed(&chart, cx, vec![a]);
    hover_at(cx, &chart, t, 0.0);
    assert!(overlay_is_current(&chart, cx));
    let before = chart.read_with(cx, |c, _| c.x_of_time(t)).unwrap();
    let rows = |chart: &Entity<Chart>, cx: &mut VisualTestContext| {
        let palette = cx.update(|_, cx| ChartPalette::from_tokens(&cx.tokens()));
        chart.read_with(cx, |c, _| c.hover_rows(&palette))
    };
    let dot_before = rows(&chart, cx)[0].at.x;

    host.update(cx, |host, cx| {
        host.width = 400.0;
        cx.notify();
    });
    cx.run_until_parked();
    let after = chart.read_with(cx, |c, _| c.x_of_time(t)).unwrap();
    assert_ne!(before, after, "the resize moved the sample");
    eprintln!(
        "DBG {:?} {:?}",
        chart.read_with(cx, |c, _| c.hovered_time()),
        chart.read_with(cx, |c, _| (
            c.cache.borrow().overlay_build,
            c.cache.borrow().build
        ))
    );
    assert!(overlay_is_current(&chart, cx), "the tooltip was re-placed");
    assert_ne!(dot_before, rows(&chart, cx)[0].at.x);
}
