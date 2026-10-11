//! [`Chart`]: the line, area and bar chart view.

use super::cache::LayoutCache;
use super::data::{Band, Series, Threshold, TimeAxis, YRange};
use super::format::ValueFormat;
use super::hover;
use super::legend::legend;
use super::paint;
use super::palette::ChartPalette;
use super::placeholder::NoData;
use super::spec::{ChartKind, Spec};
use crate::layout::v_flex;
use crate::tokens::ActiveTokens as _;
use crate::u;
use gpui::{
    Context, ElementId, InteractiveElement as _, IntoElement, MouseMoveEvent, ParentElement as _,
    Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _, Window, canvas, div,
    prelude::FluentBuilder as _, px,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// How far past a series' first and last sample (in pixels) the hover still answers.
const EDGE_PX: f64 = 12.0;

/// What a [`Chart`] is showing.
#[derive(Clone, Debug, PartialEq)]
pub enum ChartState {
    /// The plot.
    Plot,
    /// The "No data" placeholder, with the reason when one was given
    /// ([`Chart::set_unavailable`]).
    NoData(Option<SharedString>),
}

/// A time-series chart: lines, areas or bars over a [`TimeAxis`], with a value axis, gridlines,
/// optional thresholds and bands, a legend that switches series on and off, a hover tooltip and an
/// explicit "No data" state.
///
/// Create it with `cx.new(|_| Chart::line("cpu"))` and feed it with [`Chart::set_series`] whenever
/// new data arrives; it redraws only when the data, size, zoom or theme actually changed. Hover
/// state lives here: moving the cursor re-renders this entity only, and only when the snapped
/// sample changes. The chart fills its parent; give the parent a height.
pub struct Chart {
    pub(super) id: ElementId,
    pub(super) spec: Arc<Spec>,
    /// Bumped on every change of `spec`; the geometry cache keys on it.
    pub(super) generation: u64,
    pub(super) label: Option<SharedString>,
    pub(super) legend: bool,
    pub(super) unavailable: Option<SharedString>,
    /// The snapped sample time under the cursor.
    pub(super) hovered: Option<f64>,
    pub(super) cache: Rc<RefCell<LayoutCache>>,
}

impl Chart {
    /// An empty chart of `kind`. `id` must be unique among the chart's siblings.
    pub fn new(kind: ChartKind, id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            spec: Arc::new(Spec::new(kind)),
            generation: 0,
            label: None,
            legend: true,
            unavailable: None,
            hovered: None,
            cache: Rc::default(),
        }
    }

    /// An empty line chart.
    pub fn line(id: impl Into<ElementId>) -> Self {
        Self::new(ChartKind::Line, id)
    }

    /// An empty area chart.
    pub fn area(id: impl Into<ElementId>) -> Self {
        Self::new(ChartKind::Area, id)
    }

    /// An empty bar chart.
    pub fn bar(id: impl Into<ElementId>) -> Self {
        Self::new(ChartKind::Bar, id)
    }

    /// The accessible name of the chart ("CPU, last hour").
    #[must_use]
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Shows or hides the legend (shown by default when there is more than one series).
    #[must_use]
    pub fn legend(mut self, show: bool) -> Self {
        self.legend = show;
        self
    }

    /// The unit of the value axis and the tooltip.
    #[must_use]
    pub fn format(mut self, format: ValueFormat) -> Self {
        self.edit(|spec| spec.format = format);
        self
    }

    /// Fixes the value axis (for example `0..=100` for percentages) instead of fitting the data.
    #[must_use]
    pub fn y_range(mut self, range: YRange) -> Self {
        self.edit(|spec| spec.y_range = range);
        self
    }

    /// Reference lines, such as a pod's request and limit.
    #[must_use]
    pub fn thresholds(mut self, thresholds: Vec<Threshold>) -> Self {
        self.edit(|spec| spec.thresholds = thresholds);
        self
    }

    /// Shaded warn and critical zones.
    #[must_use]
    pub fn bands(mut self, bands: Vec<Band>) -> Self {
        self.edit(|spec| spec.bands = bands);
        self
    }

    /// The kind of chart.
    pub fn kind(&self) -> ChartKind {
        self.spec.kind
    }

    /// Replaces the data and the visible time range. Does nothing (no redraw) when both are
    /// the same samples (pointer-equal) and range as before, so calling it on every poll is free.
    pub fn set_series(&mut self, series: Vec<Series>, axis: TimeAxis, cx: &mut Context<Self>) {
        let same = self.spec.axis == axis
            && self.spec.series.len() == series.len()
            && self.spec.series.iter().zip(&series).all(|(a, b)| {
                a.name == b.name && a.style == b.style && Arc::ptr_eq(&a.points, &b.points)
            });
        if same {
            return;
        }
        self.update(cx, |spec| {
            spec.series = series;
            spec.axis = axis;
            spec.refresh();
        });
    }

    /// Replaces the reference lines.
    pub fn set_thresholds(&mut self, thresholds: Vec<Threshold>, cx: &mut Context<Self>) {
        if self.spec.thresholds != thresholds {
            self.update(cx, |spec| spec.thresholds = thresholds);
        }
    }

    /// Replaces the status bands.
    pub fn set_bands(&mut self, bands: Vec<Band>, cx: &mut Context<Self>) {
        if self.spec.bands != bands {
            self.update(cx, |spec| spec.bands = bands);
        }
    }

    /// Shows the "No data" state with `reason` in place of the plot (metrics-server missing,
    /// Prometheus unreachable, RBAC denied), or goes back to plotting with `None`. Without a
    /// reason set, a chart whose series hold no readings in the range shows "No data" anyway.
    pub fn set_unavailable(&mut self, reason: Option<SharedString>, cx: &mut Context<Self>) {
        if self.unavailable != reason {
            self.unavailable = reason;
            self.hovered = None;
            cx.notify();
        }
    }

    /// Switches the series called `name` off, or on again. This is what a legend click does.
    pub fn toggle_series(&mut self, name: &SharedString, cx: &mut Context<Self>) {
        self.update(cx, |spec| {
            if let Some(at) = spec.hidden.iter().position(|n| n == name) {
                spec.hidden.remove(at);
            } else {
                spec.hidden.push(name.clone());
            }
        });
        self.hovered = None;
    }

    /// Whether the legend has switched the series called `name` off.
    pub fn is_series_hidden(&self, name: &SharedString) -> bool {
        self.spec.is_hidden(name)
    }

    /// What the chart is showing: the plot, or the "No data" state (with its reason).
    pub fn state(&self) -> ChartState {
        if self.unavailable.is_some() {
            ChartState::NoData(self.unavailable.clone())
        } else if !self.spec.has_data {
            ChartState::NoData(None)
        } else {
            ChartState::Plot
        }
    }

    /// Shows (or with `None` hides) the tooltip for the sample nearest `t`, as if the cursor were
    /// there. Lets one chart's hover drive another's crosshair, and screenshots show a tooltip.
    pub fn hover_time(&mut self, t: Option<f64>, cx: &mut Context<Self>) {
        let next = t.and_then(|t| hover::snap(&self.spec, t, f64::INFINITY));
        if next != self.hovered {
            self.hovered = next;
            cx.notify();
        }
    }

    /// The time (seconds since the epoch) of the sample the tooltip is showing, if any.
    pub fn hovered_time(&self) -> Option<f64> {
        self.hovered
    }

    /// How many times the geometry was built (it should not grow while the chart just repaints).
    pub fn geometry_builds(&self) -> usize {
        self.cache.borrow().builds
    }

    /// The canvas origin in window coordinates and the plot rectangle relative to it, once the
    /// chart has painted.
    pub fn plot_bounds(&self) -> Option<gpui::Bounds<gpui::Pixels>> {
        let cache = self.cache.borrow();
        let layout = cache.layout.as_ref().filter(|l| l.drawable)?;
        Some(gpui::Bounds::new(
            cache.origin + layout.plot.origin,
            layout.plot.size,
        ))
    }

    /// The window x of time `t` once the chart has painted.
    pub fn x_of_time(&self, t: f64) -> Option<gpui::Pixels> {
        let cache = self.cache.borrow();
        let layout = cache.layout.as_ref().filter(|l| l.drawable)?;
        Some(cache.origin.x + px(layout.x_of(t)))
    }

    fn edit(&mut self, change: impl FnOnce(&mut Spec)) {
        change(Arc::make_mut(&mut self.spec));
        self.generation += 1;
    }

    fn update(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Spec)) {
        self.edit(change);
        // The hovered sample may no longer exist.
        self.hovered = None;
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let next = {
            let cache = self.cache.borrow();
            cache
                .layout
                .as_ref()
                .filter(|l| l.drawable)
                .and_then(|layout| {
                    let x = (event.position.x - cache.origin.x).as_f32();
                    let t = layout.time_at(x)?;
                    hover::snap(&self.spec, t, EDGE_PX * layout.seconds_per_pixel())
                })
        };
        if next != self.hovered {
            self.hovered = next;
            cx.notify();
        }
    }

    fn clear_hover(&mut self, cx: &mut Context<Self>) {
        if self.hovered.take().is_some() {
            cx.notify();
        }
    }

    fn summary(&self) -> SharedString {
        let name = self.label.clone().unwrap_or_else(|| "Chart".into());
        match self.state() {
            ChartState::NoData(Some(reason)) => format!("{name}: no data, {reason}").into(),
            ChartState::NoData(None) => format!("{name}: no data").into(),
            ChartState::Plot => {
                let shown = self.spec.visible().count();
                format!("{name}: {shown} series").into()
            }
        }
    }
}

impl Render for Chart {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let palette = ChartPalette::from_tokens(&tokens);
        let root = v_flex()
            .id(self.id.clone())
            .size_full()
            .role(Role::Image)
            .aria_label(self.summary());
        if let ChartState::NoData(reason) = self.state() {
            return root.child(NoData::new(reason)).into_any_element();
        }

        let cache = self.cache.clone();
        let spec = self.spec.clone();
        let generation = self.generation;
        let plot = div()
            .id(ElementId::NamedChild(
                Arc::new(self.id.clone()),
                "plot".into(),
            ))
            .debug_selector(|| "chart-plot".into())
            .relative()
            .flex_1()
            .min_h(u(px(24.)))
            .w_full()
            .on_mouse_move(cx.listener(|chart, event: &MouseMoveEvent, _, cx| {
                chart.on_mouse_move(event, cx);
            }))
            .on_hover(cx.listener(|chart, hovered: &bool, _, cx| {
                if !hovered {
                    chart.clear_hover(cx);
                }
            }))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, (), window, cx| {
                        paint::paint(
                            &mut cache.borrow_mut(),
                            generation,
                            &spec,
                            &palette,
                            bounds,
                            window,
                            cx,
                        );
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(self.overlay(&palette));

        let show_legend = self.legend && self.spec.series.len() > 1;
        root.when(show_legend, |root| {
            root.child(legend(&self.id, &self.spec, &palette, &tokens, cx))
        })
        .child(plot)
        .into_any_element()
    }
}
