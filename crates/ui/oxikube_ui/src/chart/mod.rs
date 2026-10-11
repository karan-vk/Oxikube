//! Charts for metrics: time-series line, area and bar charts, and the compact sparkline.
//!
//! Views never touch gpui-component's chart types (ADR 0004): they build [`Series`] of plain
//! [`Point`]s and hand them to a [`Chart`] entity or a [`Sparkline`] element, which draw with
//! gpui-component's plot primitives (scales' grid, paths cache, tooltip overlay) under Oxikube's
//! tokens. A gpui-component bump touches this module only.
//!
//! ```ignore
//! let cpu = cx.new(|_| Chart::area("pod-cpu").format(ValueFormat::Cores).label("CPU"));
//! cpu.update(cx, |chart, cx| {
//!     chart.set_series(
//!         vec![Series::new("nginx", points.clone())], // points: Arc<[Point]>
//!         TimeAxis::last(3600.0, now),
//!         cx,
//!     );
//! });
//! // in a render: div().h(px(160.)).child(cpu.clone())
//! ```
//!
//! What is here:
//! - [`Chart`] (an `Entity` you keep): line, area and bar over a [`TimeAxis`], value axis with nice
//!   ticks in a [`ValueFormat`], gridlines, [`Threshold`] lines and [`Band`]s, a legend whose
//!   entries switch series off and on, a hover tooltip with the time and each series' value, and an
//!   explicit "No data" state ([`Chart::set_unavailable`]) that carries a reason.
//! - [`Sparkline`] (an element built per frame): a trend line with no axes, for table cells,
//!   tiles and pulses.
//! - [`ChartPalette`]: the colours, derived from the active tokens (a categorical palette for
//!   per-container series, status tones for warn and critical).
//! - [`gallery::ChartGallery`]: every state on one page.
//!
//! # Cost
//!
//! Decimating (to the pixel columns, keeping every spike), projecting, choosing ticks, measuring
//! labels and tessellating happen when the data, the size, the zoom or the theme changes
//! (the `cache` module); a repaint of an unchanged chart is a handful of quads plus one translated copy of
//! each cached path (GPUI scales a path it is given, so that copy is the floor). Hover re-renders
//! the chart entity only, and only when the snapped sample changes. Charts run no animation, so
//! reduce-motion has nothing to switch off.

mod cache;
mod data;
mod downsample;
mod format;
pub mod gallery;
mod hover;
mod layout;
mod legend;
mod overlay;
mod paint;
mod palette;
mod paths;
mod placeholder;
mod spark_state;
mod sparkline;
mod spec;
mod ticks;
mod view;

pub use data::{Band, Point, Series, SeriesColor, SeriesStyle, Threshold, TimeAxis, Tone, YRange};
pub use downsample::{Decimated, bucket_max, decimate};
pub use format::ValueFormat;
pub use overlay::HoverRow;
pub use palette::{CATEGORICAL, ChartPalette};
pub use placeholder::NoData;
pub use sparkline::Sparkline;
pub use spec::ChartKind;
pub use ticks::{TimeTick, ValueTicks, time_ticks, tooltip_time, value_labels, value_ticks};
pub use view::{Chart, ChartState};

#[cfg(test)]
mod tests;
