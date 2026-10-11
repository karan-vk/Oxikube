//! What a chart shows: the immutable snapshot geometry is built from.

use super::data::{Band, Series, Threshold, TimeAxis, YRange};
use super::format::ValueFormat;
use gpui::SharedString;

/// The three ways to draw a [`Series`](super::Series) on a time axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChartKind {
    /// A stroked line per series.
    Line,
    /// A line with a translucent fill down to the baseline.
    Area,
    /// Vertical bars, grouped side by side when there are several series.
    Bar,
}

/// A chart's data and options. Replaced wholesale when anything changes (never edited in place),
/// so the geometry cache keys on a generation counter and paints are cheap to validate.
#[derive(Clone, Debug)]
pub(super) struct Spec {
    pub kind: ChartKind,
    pub series: Vec<Series>,
    pub axis: TimeAxis,
    pub y_range: YRange,
    pub format: ValueFormat,
    pub thresholds: Vec<Threshold>,
    pub bands: Vec<Band>,
    /// Names of the series the legend switched off.
    pub hidden: Vec<SharedString>,
    /// Whether any series has a reading inside the time axis.
    pub has_data: bool,
}

impl Spec {
    pub fn new(kind: ChartKind) -> Self {
        Self {
            kind,
            series: Vec::new(),
            axis: TimeAxis::utc(0.0, 1.0),
            y_range: YRange::Auto,
            format: ValueFormat::Number,
            thresholds: Vec::new(),
            bands: Vec::new(),
            hidden: Vec::new(),
            has_data: false,
        }
    }

    pub fn is_hidden(&self, name: &SharedString) -> bool {
        self.hidden.contains(name)
    }

    /// The series the legend has not switched off, with their positions (which pick the colour).
    pub fn visible(&self) -> impl Iterator<Item = (usize, &Series)> {
        self.series
            .iter()
            .enumerate()
            .filter(|(_, s)| !self.is_hidden(&s.name))
    }

    /// Recomputes [`Spec::has_data`] after the series or the axis changed.
    pub fn refresh(&mut self) {
        let (start, end) = (self.axis.start, self.axis.end);
        self.has_data = self.series.iter().any(|s| {
            let from = s.points.partition_point(|p| p.t < start);
            s.points[from..]
                .iter()
                .take_while(|p| p.t <= end)
                .any(|p| !p.is_gap())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::Point;

    #[test]
    fn data_outside_the_axis_is_no_data() {
        let mut spec = Spec::new(ChartKind::Line);
        spec.series = vec![Series::new("a", vec![Point::new(100.0, 1.0)])];
        spec.axis = TimeAxis::utc(0.0, 50.0);
        spec.refresh();
        assert!(!spec.has_data);
        spec.axis = TimeAxis::utc(0.0, 150.0);
        spec.refresh();
        assert!(spec.has_data);
    }

    #[test]
    fn visible_skips_hidden_series_but_keeps_positions() {
        let mut spec = Spec::new(ChartKind::Line);
        spec.series = vec![
            Series::new("a", Vec::<Point>::new()),
            Series::new("b", Vec::<Point>::new()),
            Series::new("c", Vec::<Point>::new()),
        ];
        spec.hidden = vec!["b".into()];
        let visible: Vec<usize> = spec.visible().map(|(i, _)| i).collect();
        assert_eq!(visible, [0, 2]);
    }
}
